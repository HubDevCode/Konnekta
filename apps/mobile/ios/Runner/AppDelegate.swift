import AVFoundation
import Flutter
import Photos
import UIKit

/// iOS integration. Platform limits (documented in docs/PIATTAFORME.md):
/// - third-party apps cannot read other apps' notifications or control calls;
/// - the clipboard is only readable while the app is in the foreground;
/// - background networking is limited, so Konnekta works while it is open.
@main
@objc class AppDelegate: FlutterAppDelegate {
  private var sink: FlutterEventSink?
  private let engine = AVAudioEngine()
  private var micRunning = false
  private var player: AVAudioPlayerNode?
  private var playerEngine: AVAudioEngine?
  private var playerFormat: AVAudioFormat?
  private var screenshotWatch = true

  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    let controller = window?.rootViewController as! FlutterViewController
    let messenger = controller.binaryMessenger
    FlutterEventChannel(name: "konnekta/native_events", binaryMessenger: messenger).setStreamHandler(self)
    FlutterMethodChannel(name: "konnekta/native", binaryMessenger: messenger).setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result)
    }
    UIDevice.current.isBatteryMonitoringEnabled = true
    NotificationCenter.default.addObserver(forName: UIDevice.batteryLevelDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
      self?.emit(["type": "battery"].merging(self?.battery() ?? [:]) { a, _ in a })
    }
    NotificationCenter.default.addObserver(forName: UIApplication.userDidTakeScreenshotNotification, object: nil, queue: .main) { [weak self] _ in
      self?.sendLatestScreenshot()
    }
    GeneratedPluginRegistrant.register(with: self)
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  private func emit(_ e: [String: Any]) {
    DispatchQueue.main.async { self.sink?(e) }
  }

  private func battery() -> [String: Any] {
    let level = Int(max(0, UIDevice.current.batteryLevel) * 100)
    let state = UIDevice.current.batteryState
    return ["level": level, "charging": state == .charging || state == .full, "low": level <= 15]
  }

  private func handle(_ call: FlutterMethodCall, _ result: @escaping FlutterResult) {
    let args = call.arguments as? [String: Any] ?? [:]
    switch call.method {
    case "deviceName": result(UIDevice.current.name)
    case "battery": result(battery())
    case "startMic": startMic(result)
    case "stopMic":
      engine.inputNode.removeTap(onBus: 0); engine.stop(); micRunning = false; result(true)
    case "startAudioPlayback":
      startPlayer(rate: args["sampleRate"] as? Double ?? 48000, channels: args["channels"] as? Int ?? 2); result(true)
    case "playPcm":
      if let d = args["data"] as? FlutterStandardTypedData { play(d.data) }
      result(nil)
    case "stopAudioPlayback":
      player?.stop(); playerEngine?.stop(); player = nil; playerEngine = nil; result(true)
    case "setScreenshotWatch": screenshotWatch = args["enabled"] as? Bool ?? true; result(true)
    case "ring":
      AudioServicesPlaySystemSound(1304)
      AudioServicesPlaySystemSound(kSystemSoundID_Vibrate)
      result(true)
    case "startCamera", "startScreenCapture", "startDecoder", "callAction", "setDnd", "replyNotification",
         "dismissNotification", "notificationAction", "resendNotifications", "mediaControl", "muteRinger":
      // Not available on iOS (no public API) or not implemented yet.
      result(false)
    default: result(FlutterMethodNotImplemented)
    }
  }

  // MARK: microphone -> PCM s16le 48 kHz mono, 10 ms chunks

  private func startMic(_ result: @escaping FlutterResult) {
    AVAudioSession.sharedInstance().requestRecordPermission { ok in
      DispatchQueue.main.async {
        guard ok else { result(false); return }
        do {
          let session = AVAudioSession.sharedInstance()
          try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.defaultToSpeaker, .allowBluetooth])
          try session.setPreferredIOBufferDuration(0.005)
          try session.setActive(true)
          let input = self.engine.inputNode
          let target = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 48000, channels: 1, interleaved: true)!
          let converter = AVAudioConverter(from: input.outputFormat(forBus: 0), to: target)!
          var pending = Data()
          input.installTap(onBus: 0, bufferSize: 480, format: input.outputFormat(forBus: 0)) { buf, _ in
            let ratio = 48000 / buf.format.sampleRate
            guard let out = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: AVAudioFrameCount(Double(buf.frameLength) * ratio) + 16) else { return }
            var fed = false
            converter.convert(to: out, error: nil) { _, status in
              if fed { status.pointee = .noDataNow; return nil }
              fed = true; status.pointee = .haveData; return buf
            }
            pending.append(Data(bytes: out.int16ChannelData![0], count: Int(out.frameLength) * 2))
            while pending.count >= 960 {
              let chunk = pending.prefix(960)
              pending.removeFirst(960)
              self.emit(["type": "frame", "plugin": "microphone", "kind": 1, "flags": 0,
                         "pts": Int(Date().timeIntervalSince1970 * 1_000_000), "data": FlutterStandardTypedData(bytes: Data(chunk))])
            }
          }
          try self.engine.start()
          self.micRunning = true
          result(true)
        } catch {
          result(FlutterError(code: "mic", message: error.localizedDescription, details: nil))
        }
      }
    }
  }

  // MARK: computer audio playback

  private func startPlayer(rate: Double, channels: Int) {
    let e = AVAudioEngine()
    let p = AVAudioPlayerNode()
    let fmt = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: AVAudioChannelCount(channels), interleaved: false)!
    e.attach(p)
    e.connect(p, to: e.mainMixerNode, format: fmt)
    try? AVAudioSession.sharedInstance().setCategory(.playback)
    try? e.start()
    p.play()
    player = p; playerEngine = e; playerFormat = fmt
  }

  private func play(_ data: Data) {
    guard let p = player, let fmt = playerFormat else { return }
    let ch = Int(fmt.channelCount)
    let frames = data.count / 2 / ch
    guard let buf = AVAudioPCMBuffer(pcmFormat: fmt, frameCapacity: AVAudioFrameCount(frames)) else { return }
    buf.frameLength = AVAudioFrameCount(frames)
    data.withUnsafeBytes { raw in
      let s = raw.bindMemory(to: Int16.self)
      for f in 0..<frames { for c in 0..<ch { buf.floatChannelData![c][f] = Float(s[f * ch + c]) / 32768 } }
    }
    p.scheduleBuffer(buf, completionHandler: nil)
  }

  // MARK: screenshots

  private func sendLatestScreenshot() {
    guard screenshotWatch else { return }
    PHPhotoLibrary.requestAuthorization(for: .readWrite) { status in
      guard status == .authorized || status == .limited else { return }
      DispatchQueue.global().asyncAfter(deadline: .now() + 1.0) {
        let opts = PHFetchOptions()
        opts.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        opts.fetchLimit = 1
        opts.predicate = NSPredicate(format: "(mediaSubtype & %d) != 0", PHAssetMediaSubtype.photoScreenshot.rawValue)
        guard let asset = PHAsset.fetchAssets(with: .image, options: opts).firstObject else { return }
        PHImageManager.default().requestImageDataAndOrientation(for: asset, options: nil) { data, _, _, _ in
          guard let data = data, let img = UIImage(data: data), let png = img.pngData() else { return }
          let url = FileManager.default.temporaryDirectory.appendingPathComponent("Screenshot_\(Int(Date().timeIntervalSince1970)).png")
          try? png.write(to: url)
          self.emit(["type": "screenshot", "path": url.path])
        }
      }
    }
  }
}

extension AppDelegate: FlutterStreamHandler {
  func onListen(withArguments arguments: Any?, eventSink events: @escaping FlutterEventSink) -> FlutterError? {
    sink = events
    emit(["type": "battery"].merging(battery()) { a, _ in a })
    return nil
  }

  func onCancel(withArguments arguments: Any?) -> FlutterError? {
    sink = nil
    return nil
  }
}
