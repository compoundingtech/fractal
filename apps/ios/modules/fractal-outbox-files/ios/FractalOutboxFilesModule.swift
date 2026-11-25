import CryptoKit
import Darwin
import ExpoModulesCore
import Foundation

private let outboxFilesQueue = DispatchQueue(label: "fractal.outbox.files")

public final class FractalOutboxFilesModule: Module {
  public func definition() -> ModuleDefinition {
    Name("FractalOutboxFiles")

    AsyncFunction("prepareStorage") { () -> [String: Any] in
      let (root, spool) = try self.protectedDirectories()
      return ["databaseDirectory": root.path, "spoolDirectory": spool.path,
              "databaseExcludedFromBackup": true, "spoolExcludedFromBackup": true]
    }.runOnQueue(outboxFilesQueue)

    AsyncFunction("writeAttachment") { (base64: String, contentHash: String) -> [String: Any] in
      let (_, spool) = try self.protectedDirectories()
      let name = try self.hashName(contentHash)
      guard let bytes = Data(base64Encoded: base64), bytes.count <= 10 * 1024 * 1024,
            self.hash(bytes) == name else { throw self.failure("Invalid attachment bytes or digest") }
      let destination = spool.appendingPathComponent(name)
      if FileManager.default.fileExists(atPath: destination.path) {
        try self.verifyFile(destination, hash: name)
      } else {
        let temporary = spool.appendingPathComponent(".\(UUID().uuidString).partial")
        // Acceptance removes only unaccepted temporary files. Final files are removed
        // separately after JS proves terminal retention and absence of live references.
        defer { try? FileManager.default.removeItem(at: temporary) }
        try bytes.write(to: temporary, options: .withoutOverwriting)
        try FileManager.default.setAttributes([.posixPermissions: 0o600, .protectionKey: FileProtectionType.complete], atPath: temporary.path)
        let handle = try FileHandle(forWritingTo: temporary)
        do { try handle.synchronize(); try handle.close() }
        catch { try? handle.close(); throw error }
        let linked = temporary.path.withCString { source in
          destination.path.withCString { target in Darwin.link(source, target) }
        }
        if linked != 0 && errno != EEXIST { throw self.posixFailure() }
        try self.verifyFile(destination, hash: name)
      }
      try self.synchronizeDirectory(spool)
      return ["reference": "spool/\(name)", "contentHash": "sha256:\(name)", "bytes": bytes.count]
    }.runOnQueue(outboxFilesQueue)

    AsyncFunction("readAttachment") { (reference: String) -> String in
      let (_, spool) = try self.protectedDirectories()
      guard reference.hasPrefix("spool/") else { throw self.failure("Invalid spool reference") }
      let name = try self.hashName("sha256:" + String(reference.dropFirst(6)))
      let url = spool.appendingPathComponent(name)
      return try self.verifyFile(url, hash: name).base64EncodedString()
    }.runOnQueue(outboxFilesQueue)

    // JS queues only proven, unreferenced terminal files while acceptance is excluded.
    AsyncFunction("deleteAttachment") { (reference: String) in
      let (_, spool) = try self.protectedDirectories()
      guard reference.hasPrefix("spool/") else { throw self.failure("Invalid spool reference") }
      let name = try self.hashName("sha256:" + String(reference.dropFirst(6)))
      let url = spool.appendingPathComponent(name)
      if FileManager.default.fileExists(atPath: url.path) {
        try FileManager.default.removeItem(at: url)
      }
      try self.synchronizeDirectory(spool)
    }.runOnQueue(outboxFilesQueue)
  }

  private func protectedDirectories() throws -> (URL, URL) {
    guard let documents = appContext?.config.documentDirectory else { throw failure("Private documents directory unavailable") }
    // Same Documents/SQLite root as the pinned Expo SQLite defaultDatabaseDirectory.
    let root = documents.appendingPathComponent("SQLite/fractal-outbox", isDirectory: true)
    let spool = root.appendingPathComponent("spool", isDirectory: true)
    for original in [root, spool] {
      try FileManager.default.createDirectory(at: original, withIntermediateDirectories: true,
        attributes: [.posixPermissions: 0o700, .protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
      let attributes = try original.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
      guard attributes.isDirectory == true && attributes.isSymbolicLink != true else { throw failure("Invalid private outbox directory") }
      // Upgrade existing roots too; createDirectory attributes only apply on creation.
      try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], atPath: original.path)
      var url = original
      var values = URLResourceValues()
      values.isExcludedFromBackup = true // NSURLIsExcludedFromBackupKey
      try url.setResourceValues(values)
      let readback = URL(fileURLWithPath: original.path, isDirectory: true)
      guard try readback.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true else {
        throw failure("Outbox backup exclusion read-back failed")
      }
      try synchronizeDirectory(original)
      try synchronizeDirectory(original.deletingLastPathComponent())
    }
    return (root, spool)
  }

  private func hashName(_ value: String) throws -> String {
    guard value.hasPrefix("sha256:") else { throw failure("Invalid attachment digest") }
    let name = String(value.dropFirst(7))
    guard name.utf8.count == 64 && name.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
      throw failure("Invalid attachment digest")
    }
    return name
  }

  private func hash(_ bytes: Data) -> String {
    SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
  }

  @discardableResult
  private func verifyFile(_ url: URL, hash expected: String) throws -> Data {
    let values = try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
    guard values.isRegularFile == true && values.isSymbolicLink != true,
          let size = values.fileSize, size <= 10 * 1024 * 1024 else { throw failure("Invalid spool file") }
    let bytes = try Data(contentsOf: url)
    guard hash(bytes) == expected else { throw failure("Spool content digest mismatch") }
    return bytes
  }

  private func synchronizeDirectory(_ url: URL) throws {
    let descriptor = Darwin.open(url.path, O_RDONLY)
    guard descriptor >= 0 else { throw posixFailure() }
    defer { Darwin.close(descriptor) }
    guard Darwin.fsync(descriptor) == 0 else { throw posixFailure() }
  }

  private func posixFailure() -> POSIXError { POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
  private func failure(_ message: String) -> Exception { Exception(name: "OutboxStorageUnavailable", description: message) }
}
