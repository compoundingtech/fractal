import AVFoundation
import ExpoModulesCore
import UIKit

public final class FractalPairingScannerModule: Module {
  private var scanning = false

  public func definition() -> ModuleDefinition {
    Name("FractalPairingScanner")
    AsyncFunction("scan") { (promise: Promise) in
      guard !self.scanning else {
        promise.reject("ERR_SCANNER_BUSY", "A scanner is already open.")
        return
      }
      self.scanning = true
      let open: (Bool) -> Void = { allowed in
        DispatchQueue.main.async {
          guard allowed else {
            self.scanning = false
            promise.reject("ERR_CAMERA_PERMISSION_DENIED", "Camera access is off. Enter pairing details manually.")
            return
          }
          guard let presenter = self.appContext?.utilities?.currentViewController(), presenter.viewIfLoaded?.window != nil else {
            self.scanning = false
            promise.reject("ERR_SCANNER_UNAVAILABLE", "The scanner could not open.")
            return
          }
          let scanner = PairingScannerController()
          scanner.modalPresentationStyle = .fullScreen
          scanner.completed = { value, failed in
            scanner.dismiss(animated: true) {
              self.scanning = false
              if failed {
                promise.reject("ERR_SCANNER_UNAVAILABLE", "The camera could not scan. Enter pairing details manually.")
              } else if let value {
                promise.resolve(value)
              } else {
                promise.resolve(nil as String?)
              }
            }
          }
          presenter.present(scanner, animated: true)
        }
      }
      switch AVCaptureDevice.authorizationStatus(for: .video) {
      case .authorized: open(true)
      case .notDetermined: AVCaptureDevice.requestAccess(for: .video, completionHandler: open)
      default: open(false)
      }
    }.runOnQueue(.main)
  }
}

private final class PairingScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
  var completed: ((String?, Bool) -> Void)?
  private let session = AVCaptureSession()
  private let captureQueue = DispatchQueue(label: "com.compoundingtech.fractal.pairing-scanner")
  private var preview: AVCaptureVideoPreviewLayer?
  private var finished = false

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .black
    let layer = AVCaptureVideoPreviewLayer(session: session)
    layer.videoGravity = .resizeAspectFill
    view.layer.addSublayer(layer)
    preview = layer

    let cancel = UIButton(type: .system)
    cancel.setTitle("Cancel", for: .normal)
    cancel.setTitleColor(.white, for: .normal)
    cancel.backgroundColor = UIColor.black.withAlphaComponent(0.7)
    cancel.layer.cornerRadius = 12
    cancel.titleLabel?.font = .preferredFont(forTextStyle: .headline)
    cancel.titleLabel?.adjustsFontForContentSizeCategory = true
    cancel.contentEdgeInsets = UIEdgeInsets(top: 14, left: 24, bottom: 14, right: 24)
    cancel.translatesAutoresizingMaskIntoConstraints = false
    cancel.addTarget(self, action: #selector(cancelScan), for: .touchUpInside)
    view.addSubview(cancel)

    let instruction = UILabel()
    instruction.text = "Scan the pairing QR shown by fractal devices pair"
    instruction.textColor = .white
    instruction.backgroundColor = UIColor.black.withAlphaComponent(0.7)
    instruction.font = .preferredFont(forTextStyle: .body)
    instruction.adjustsFontForContentSizeCategory = true
    instruction.numberOfLines = 0
    instruction.textAlignment = .center
    instruction.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(instruction)
    NSLayoutConstraint.activate([
      cancel.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
      cancel.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
      instruction.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
      instruction.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -20),
      instruction.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -32),
    ])
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    captureQueue.async { [self] in
      guard let camera = AVCaptureDevice.default(for: .video),
        let input = try? AVCaptureDeviceInput(device: camera), session.canAddInput(input) else {
        DispatchQueue.main.async { self.finish(nil, failed: true) }
        return
      }
      session.beginConfiguration()
      session.addInput(input)
      let output = AVCaptureMetadataOutput()
      guard session.canAddOutput(output) else {
        session.commitConfiguration()
        DispatchQueue.main.async { self.finish(nil, failed: true) }
        return
      }
      session.addOutput(output)
      output.setMetadataObjectsDelegate(self, queue: .main)
      guard output.availableMetadataObjectTypes.contains(.qr) else {
        session.commitConfiguration()
        DispatchQueue.main.async { self.finish(nil, failed: true) }
        return
      }
      output.metadataObjectTypes = [.qr]
      session.commitConfiguration()
      session.startRunning()
    }
  }

  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    preview?.frame = view.bounds
  }

  @objc private func cancelScan() { finish(nil) }

  private func finish(_ value: String?, failed: Bool = false) {
    guard !finished else { return }
    finished = true
    captureQueue.async { self.session.stopRunning() }
    let completion = completed
    completed = nil
    completion?(value, failed)
  }

  func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput metadataObjects: [AVMetadataObject], from connection: AVCaptureConnection) {
    guard let value = metadataObjects.compactMap({ ($0 as? AVMetadataMachineReadableCodeObject)?.stringValue }).first else { return }
    finish(value)
  }
}
