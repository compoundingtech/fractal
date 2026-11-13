import AttentionCore
import Carbon.HIToolbox

/// The global Stop hotkey, ⌃⌥⌘. (Control-Option-Command-Period): stops the item being spoken
/// from any app. Carbon's `RegisterEventHotKey` needs no Accessibility or Input Monitoring
/// permission and sees only this key combination.
@MainActor
final class StopHotkey {
  private var hotKey: EventHotKeyRef?
  private var handler: EventHandlerRef?
  private let action: @MainActor () -> Void

  init(action: @escaping @MainActor () -> Void) { self.action = action }

  /// Returns why the hotkey could not be registered, e.g. another app holds it.
  func register() -> String? {
    var type = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
    let context = Unmanaged.passUnretained(self).toOpaque()
    let installed = InstallEventHandler(
      GetApplicationEventTarget(),
      { _, _, context in
        guard let context else { return OSStatus(eventNotHandledErr) }
        let hotkey = Unmanaged<StopHotkey>.fromOpaque(context).takeUnretainedValue()
        MainActor.assumeIsolated { hotkey.action() }
        return noErr
      }, 1, &type, context, &handler)
    guard installed == noErr else { return "event handler \(installed)" }
    let id = EventHotKeyID(signature: OSType(0x6174_7370), id: 1)  // 'atsp'
    let status = RegisterEventHotKey(
      UInt32(kVK_ANSI_Period), UInt32(controlKey | optionKey | cmdKey), id, GetApplicationEventTarget(), 0, &hotKey)
    return status == noErr ? nil : "\(SpeechHotkey.stop) is taken (\(status))"
  }

  func unregister() {
    if let hotKey { UnregisterEventHotKey(hotKey) }
    if let handler { RemoveEventHandler(handler) }
    hotKey = nil
    handler = nil
  }
}
