import AttentionCore
import CoreAudio
import CoreMediaIO
import Foundation

/// Tells the speech coordinator which microphones and cameras other processes use: a call,
/// for speech. Event-driven, no polling and no permission prompts:
/// - Microphone: Core Audio's process objects (`kAudioHardwarePropertyProcessObjectList`), one
///   listener on the list and one on each process's `kAudioProcessPropertyIsRunningInput`, the
///   same signal as the menu bar's microphone indicator. Per process, so the speech service's
///   own output on a headset never counts as a call.
/// - Camera: each Core Media IO device's `kCMIODevicePropertyDeviceIsRunningSomewhere`, with a
///   listener on the device list for cameras plugged in later.
/// A call ends only after `endGrace` without any of them, so a brief input restart (a device
/// switch mid-call) neither ends the call nor resets the person's per-call override.
@MainActor
final class CallMonitor {
  static let endGrace: Duration = .seconds(15)

  private let report: @MainActor ([String]) -> Void
  private let log: (String) -> Void
  private let queue = DispatchQueue(label: "attention-tray.call-monitor")
  private var audioListeners: [(AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertyListenerBlock)] = []
  private var cameraListeners: [(CMIOObjectID, CMIOObjectPropertyAddress, CMIOObjectPropertyListenerBlock)] = []
  private var reported: [String] = []
  private var ending: Task<Void, Never>?

  init(report: @escaping @MainActor ([String]) -> Void, log: @escaping (String) -> Void) {
    self.report = report
    self.log = log
  }

  func start() {
    var processes = Self.address(kAudioHardwarePropertyProcessObjectList)
    listenAudio(AudioObjectID(kAudioObjectSystemObject), &processes)
    var devices = CMIOObjectPropertyAddress(
      mSelector: CMIOObjectPropertySelector(kCMIOHardwarePropertyDevices),
      mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeGlobal),
      mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementMain))
    listenCamera(CMIOObjectID(kCMIOObjectSystemObject), &devices)
    rescan()
  }

  func stop() {
    for (object, address, block) in audioListeners {
      var address = address
      AudioObjectRemovePropertyListenerBlock(object, &address, queue, block)
    }
    for (object, address, block) in cameraListeners {
      var address = address
      CMIOObjectRemovePropertyListenerBlock(object, &address, queue, block)
    }
    audioListeners = []
    cameraListeners = []
    ending?.cancel()
  }

  /// Re-read every process and camera, listening to any not yet listened to and dropping the
  /// listeners of processes that left.
  private func rescan() {
    let own = getpid()
    var inUse: [String] = []
    let processes = Self.audioProcesses()
    let system = AudioObjectID(kAudioObjectSystemObject)
    audioListeners.removeAll { object, address, block in
      guard object != system, !processes.contains(object) else { return false }
      var address = address
      AudioObjectRemovePropertyListenerBlock(object, &address, queue, block)
      return true
    }
    for process in processes {
      if !audioListeners.contains(where: { $0.0 == process }) {
        var running = Self.address(kAudioProcessPropertyIsRunningInput)
        listenAudio(process, &running)
      }
      guard Self.uint32(process, kAudioProcessPropertyIsRunningInput) == 1 else { continue }
      guard let pid = Self.pid(process), pid != own else { continue }
      inUse.append("microphone: \(Self.bundleID(process) ?? "pid \(pid)")")
    }
    for camera in Self.cameras() {
      var running = CMIOObjectPropertyAddress(
        mSelector: CMIOObjectPropertySelector(kCMIODevicePropertyDeviceIsRunningSomewhere),
        mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeWildcard),
        mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementWildcard))
      if !cameraListeners.contains(where: { $0.0 == camera }) { listenCamera(camera, &running) }
      if Self.cameraRunning(camera) { inUse.append("camera: \(Self.cameraName(camera) ?? "camera \(camera)")") }
    }
    apply(inUse.sorted())
  }

  private func apply(_ inUse: [String]) {
    if !inUse.isEmpty {
      ending?.cancel()
      ending = nil
      if inUse != reported { send(inUse) }
    } else if !reported.isEmpty, ending == nil {
      ending = Task { [weak self] in
        do { try await Task.sleep(for: Self.endGrace) } catch { return }
        self?.ending = nil
        self?.send([])
      }
    }
  }

  private func send(_ inUse: [String]) {
    reported = inUse
    report(inUse)
  }

  private func listenAudio(_ object: AudioObjectID, _ address: inout AudioObjectPropertyAddress) {
    let block: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
      DispatchQueue.main.async { MainActor.assumeIsolated { self?.rescan() } }
    }
    let status = AudioObjectAddPropertyListenerBlock(object, &address, queue, block)
    if status == noErr { audioListeners.append((object, address, block)) } else { log("call monitor: audio listener \(status)") }
  }

  private func listenCamera(_ object: CMIOObjectID, _ address: inout CMIOObjectPropertyAddress) {
    let block: CMIOObjectPropertyListenerBlock = { [weak self] _, _ in
      DispatchQueue.main.async { MainActor.assumeIsolated { self?.rescan() } }
    }
    let status = CMIOObjectAddPropertyListenerBlock(object, &address, queue, block)
    if status == noErr { cameraListeners.append((object, address, block)) } else { log("call monitor: camera listener \(status)") }
  }

  // MARK: Core Audio

  nonisolated static func address(_ selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(
      mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
  }

  nonisolated static func audioProcesses() -> [AudioObjectID] {
    var address = address(kAudioHardwarePropertyProcessObjectList)
    var size: UInt32 = 0
    let system = AudioObjectID(kAudioObjectSystemObject)
    guard AudioObjectGetPropertyDataSize(system, &address, 0, nil, &size) == noErr, size > 0 else { return [] }
    var ids = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
    guard AudioObjectGetPropertyData(system, &address, 0, nil, &size, &ids) == noErr else { return [] }
    return ids
  }

  nonisolated static func uint32(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> UInt32? {
    var address = address(selector)
    var value: UInt32 = 0
    var size = UInt32(MemoryLayout<UInt32>.size)
    return AudioObjectGetPropertyData(object, &address, 0, nil, &size, &value) == noErr ? value : nil
  }

  nonisolated static func pid(_ process: AudioObjectID) -> pid_t? {
    var address = address(kAudioProcessPropertyPID)
    var value: pid_t = 0
    var size = UInt32(MemoryLayout<pid_t>.size)
    return AudioObjectGetPropertyData(process, &address, 0, nil, &size, &value) == noErr ? value : nil
  }

  nonisolated static func bundleID(_ process: AudioObjectID) -> String? {
    var address = address(kAudioProcessPropertyBundleID)
    var value: Unmanaged<CFString>?
    var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
    guard AudioObjectGetPropertyData(process, &address, 0, nil, &size, &value) == noErr, let value else { return nil }
    let id = value.takeRetainedValue() as String
    return id.isEmpty ? nil : id
  }

  // MARK: Core Media IO

  nonisolated static func cameras() -> [CMIOObjectID] {
    var address = CMIOObjectPropertyAddress(
      mSelector: CMIOObjectPropertySelector(kCMIOHardwarePropertyDevices),
      mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeGlobal),
      mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementMain))
    let system = CMIOObjectID(kCMIOObjectSystemObject)
    var size: UInt32 = 0
    guard CMIOObjectGetPropertyDataSize(system, &address, 0, nil, &size) == noErr, size > 0 else { return [] }
    var ids = [CMIOObjectID](repeating: 0, count: Int(size) / MemoryLayout<CMIOObjectID>.size)
    var used: UInt32 = 0
    guard CMIOObjectGetPropertyData(system, &address, 0, nil, size, &used, &ids) == noErr else { return [] }
    return ids
  }

  nonisolated static func cameraRunning(_ camera: CMIOObjectID) -> Bool {
    var address = CMIOObjectPropertyAddress(
      mSelector: CMIOObjectPropertySelector(kCMIODevicePropertyDeviceIsRunningSomewhere),
      mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeWildcard),
      mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementWildcard))
    var value: UInt32 = 0
    var used: UInt32 = 0
    let status = CMIOObjectGetPropertyData(camera, &address, 0, nil, UInt32(MemoryLayout<UInt32>.size), &used, &value)
    return status == noErr && value != 0
  }

  nonisolated static func cameraName(_ camera: CMIOObjectID) -> String? {
    var address = CMIOObjectPropertyAddress(
      mSelector: CMIOObjectPropertySelector(kCMIOObjectPropertyName),
      mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeGlobal),
      mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementMain))
    var value: Unmanaged<CFString>?
    var used: UInt32 = 0
    let size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
    guard CMIOObjectGetPropertyData(camera, &address, 0, nil, size, &used, &value) == noErr, let value else { return nil }
    return value.takeRetainedValue() as String
  }
}
