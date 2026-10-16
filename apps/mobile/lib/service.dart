// Phone-side application state. The Rust core handles discovery, TLS,
// pairing and permissions; this layer maps authorised packets to native
// features (Kotlin / Swift) and back.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

import 'ffi.dart';

const native = MethodChannel('konnekta/native');
const nativeEvents = EventChannel('konnekta/native_events');

class Device {
  Device(this.json);
  final Map<String, dynamic> json;
  String get id => json['id'] as String;
  String get name => json['name'] as String;
  String get type => json['deviceType'] as String? ?? 'desktop';
  bool get paired => json['paired'] == true;
  bool get connected => json['connected'] == true;
  String get pairState => json['pairState'] as String? ?? '';
  List<String> get permissions => List<String>.from(json['permissions'] ?? const []);
  List<String> get incoming => List<String>.from(json['incomingCapabilities'] ?? const []);
  bool supports(String type) => incoming.contains(type);
}

class PairPrompt {
  PairPrompt(this.deviceId, this.deviceName, this.code, {this.outgoing = false});
  final String deviceId, deviceName, code;
  final bool outgoing;
}

/// Active stream session (mic, camera, screen, desktop, system audio).
class StreamSession {
  StreamSession(this.plugin, this.deviceId, this.session, this.channel);
  final String plugin, deviceId, session;
  final int channel;
  StreamSubscription<Frame>? sub;
}

class KonnektaService extends ChangeNotifier {
  KonnektaCore? core;
  String? error;
  Map<String, dynamic> me = {};
  List<Device> devices = [];
  PairPrompt? pairPrompt;
  final messages = StreamController<String>.broadcast();

  // Per-device data coming from computers.
  final Map<String, Map<String, dynamic>> deckLayout = {};
  final Map<String, Map<String, dynamic>> deckStates = {};
  final Map<String, Map<String, dynamic>> remoteCommands = {};
  final Map<String, Map<String, dynamic>> pcMedia = {};
  final Map<String, List<String>> pcPlayers = {};
  final Map<String, StreamSession> streams = {}; // key: "$plugin/$deviceId"
  int? desktopTextureId;

  bool screenshotAutoTransfer = true;
  bool dndSync = true;
  String? _lastClipboard;

  Iterable<Device> get pairedOnline => devices.where((d) => d.paired && d.connected);

  Future<void> init() async {
    try {
      final dir = await getApplicationSupportDirectory();
      final downloads = Platform.isAndroid
          ? Directory('/storage/emulated/0/Download/Konnekta')
          : Directory('${(await getApplicationDocumentsDirectory()).path}/Received');
      String name = 'Telefono';
      try {
        name = (await native.invokeMethod<String>('deviceName')) ?? name;
      } catch (_) {}
      core = await KonnektaCore.start({
        'deviceName': name,
        'deviceType': 'phone',
        'downloadDir': downloads.path,
        // Answering / rejecting calls is only possible on Android.
        'callControl': Platform.isAndroid,
      }, dir.path);
      me = Map<String, dynamic>.from(await core!.call('info'));
      core!.events().listen(_onCoreEvent);
      nativeEvents.receiveBroadcastStream().listen((e) => _onNativeEvent(Map<String, dynamic>.from(e as Map)));
      await refresh();
    } catch (e) {
      error = e.toString();
    }
    notifyListeners();
  }

  Future<void> refresh() async {
    final list = await core!.call('devices') as List;
    devices = list.map((e) => Device(Map<String, dynamic>.from(e as Map))).toList();
    notifyListeners();
  }

  Future<dynamic> api(String method, [Map<String, dynamic>? p]) async {
    try {
      return await core!.call(method, p);
    } on KonnektaException catch (e) {
      messages.add(e.message);
      rethrow;
    }
  }

  Future<void> send(String deviceId, String type, Map<String, dynamic> body) async {
    try {
      await core!.sendPacket(deviceId, type, body);
    } on KonnektaException catch (e) {
      messages.add(e.message);
    }
  }

  Future<void> broadcast(String type, Map<String, dynamic> body) async {
    for (final d in pairedOnline) {
      if (d.supports(type)) {
        try {
          await core!.sendPacket(d.id, type, body);
        } catch (_) {/* permission not granted for this device */}
      }
    }
  }

  // ------------------------------------------------------------ core events --

  void _onCoreEvent(Map<String, dynamic> ev) {
    switch (ev['event']) {
      case 'deviceConnected':
      case 'deviceUpdated':
      case 'deviceDisconnected':
      case 'unpaired':
        refresh();
        if (ev['event'] == 'deviceDisconnected') _stopStreamsFor(ev['deviceId'] as String);
      case 'paired':
        pairPrompt = null;
        refresh();
        messages.add('Associazione completata');
        _sendInitialState();
      case 'pairRequested':
        pairPrompt = PairPrompt(ev['deviceId'], ev['deviceName'], ev['verificationKey']);
        notifyListeners();
      case 'pairOutgoing':
        final d = devices.where((x) => x.id == ev['deviceId']);
        pairPrompt = PairPrompt(ev['deviceId'], d.isEmpty ? '' : d.first.name, ev['verificationKey'], outgoing: true);
        notifyListeners();
      case 'pairRejected':
        pairPrompt = null;
        messages.add('Associazione non riuscita: ${ev['reason']}');
        refresh();
      case 'securityWarning':
        messages.add('⚠️ ${ev['message']}');
      case 'plugin':
        if (ev['kind'] == 'connected') {
          _sendInitialState();
        } else if (ev['kind'] == 'packet') {
          _onPacket(ev['deviceId'] as String, ev['plugin'] as String, Map<String, dynamic>.from(ev['data'] as Map));
        }
    }
  }

  Future<void> _sendInitialState() async {
    try {
      final b = await native.invokeMapMethod<String, dynamic>('battery');
      if (b != null) await broadcast('kdeconnect.battery', {'currentCharge': b['level'], 'isCharging': b['charging'], 'thresholdEvent': 0});
    } catch (_) {}
  }

  Future<void> _onPacket(String dev, String plugin, Map<String, dynamic> data) async {
    final type = data['type'] as String;
    final body = Map<String, dynamic>.from(data['body'] as Map? ?? {});
    switch (type) {
      case 'kdeconnect.ping':
        messages.add('Ping: ${body['message'] ?? ''}');
      case 'kdeconnect.clipboard':
      case 'kdeconnect.clipboard.connect':
        final text = body['content'] as String? ?? '';
        _lastClipboard = text;
        await Clipboard.setData(ClipboardData(text: text));
      case 'kdeconnect.share.request':
        await _onShare(dev, body, data['payloadKey'] as String?);
      case 'kdeconnect.mpris':
        if (body['playerList'] != null) pcPlayers[dev] = List<String>.from(body['playerList']);
        if (body['title'] != null || body['isPlaying'] != null) pcMedia[dev] = {...?pcMedia[dev], ...body};
        notifyListeners();
      case 'kdeconnect.mpris.request':
        await _nativeSafe('mediaControl', body);
      case 'kdeconnect.runcommand':
        remoteCommands[dev] = Map<String, dynamic>.from(await _decodeCommandList(body['commandList'] as String? ?? '{}'));
        notifyListeners();
      case 'kdeconnect.notification.request':
        if (body['request'] == true) await _nativeSafe('resendNotifications');
        if (body['cancel'] != null) await _nativeSafe('dismissNotification', {'id': body['cancel']});
      case 'kdeconnect.notification.reply':
        await _nativeSafe('replyNotification', body);
      case 'kdeconnect.notification.action':
        await _nativeSafe('notificationAction', body);
      case 'kdeconnect.findmyphone.request':
        await _nativeSafe('ring');
      case 'kdeconnect.telephony.request_mute':
        await _nativeSafe('muteRinger');
      case 'konnekta.telephony.action':
        await _nativeSafe('callAction', body);
      case 'konnekta.deck':
        if (body['type'] == 'layout') deckLayout[dev] = body;
        if (body['type'] == 'state') {
          deckStates[dev] = {...?deckStates[dev], ...Map<String, dynamic>.from(body['states'] as Map? ?? {})};
        }
        notifyListeners();
      case 'konnekta.dnd':
        if (dndSync) await _nativeSafe('setDnd', {'enabled': body['enabled']});
      case 'konnekta.screenshot':
        final cfg = body['config'];
        if (cfg is Map) {
          screenshotAutoTransfer = cfg['autoTransfer'] == true;
          await _nativeSafe('setScreenshotWatch', {'enabled': screenshotAutoTransfer});
          notifyListeners();
        }
      case 'konnekta.microphone':
      case 'konnekta.camera':
      case 'konnekta.phonescreen':
      case 'konnekta.desktop':
      case 'konnekta.systemaudio':
        await _onStreamMessage(dev, plugin, type, body);
    }
  }

  Future<Map<String, dynamic>> _decodeCommandList(String raw) async {
    try {
      return Map<String, dynamic>.from(jsonDecode(raw) as Map);
    } catch (_) {
      return {};
    }
  }

  Future<void> _onShare(String dev, Map<String, dynamic> body, String? payloadKey) async {
    if (body['text'] != null) {
      await Clipboard.setData(ClipboardData(text: body['text'] as String));
      messages.add('Testo ricevuto e copiato negli appunti');
      return;
    }
    if (body['url'] != null) {
      messages.add('Link ricevuto: ${body['url']}');
      await _nativeSafe('openUrl', {'url': body['url']});
      return;
    }
    if (payloadKey == null) return;
    final dir = Directory((await core!.call('getConfig'))['download_dir'] as String);
    await dir.create(recursive: true);
    final raw = (body['filename'] as String? ?? 'file').split(RegExp(r'[\\/]')).last;
    final safe = raw.isEmpty || raw.startsWith('.') ? 'file_$raw' : raw;
    var path = '${dir.path}/$safe';
    for (var i = 1; File(path).existsSync(); i++) {
      path = '${dir.path}/($i) $safe';
    }
    try {
      await core!.call('fetchPayload', {'payloadKey': payloadKey, 'path': path});
      messages.add('File ricevuto: $safe');
      await _nativeSafe('scanFile', {'path': path});
    } catch (e) {
      messages.add('Ricezione non riuscita: $e');
      try {
        File(path).deleteSync();
      } catch (_) {}
    }
  }

  // ---------------------------------------------------------------- streams --

  String _skey(String plugin, String dev) => '$plugin/$dev';

  Future<void> _onStreamMessage(String dev, String plugin, String type, Map<String, dynamic> m) async {
    final action = m['action'];
    final session = m['session'] as String;
    final params = Map<String, dynamic>.from(m['params'] as Map? ?? {});
    final key = _skey(plugin, dev);
    if (action == 'stop') {
      await stopStream(plugin, dev, notifyPeer: false);
      return;
    }
    // The computer asks us to produce (mic / camera / phone screen).
    if (action == 'request' && const {'microphone', 'camera', 'phonescreen'}.contains(plugin)) {
      await stopStream(plugin, dev, notifyPeer: false);
      final r = await core!.call('streamListen', {'deviceId': dev});
      final s = StreamSession(plugin, dev, session, r['channelId'] as int);
      streams[key] = s;
      final outParams = switch (plugin) {
        'microphone' => {'codec': 'pcm_s16le', 'sampleRate': 48000, 'channels': 1},
        _ => {
            'codec': 'h264',
            'width': params['width'] ?? 1280,
            'height': params['height'] ?? 720,
            'fps': params['fps'] ?? 30,
            'facing': params['facing'] ?? 'back',
          },
      };
      final ok = await _nativeSafe(switch (plugin) { 'microphone' => 'startMic', 'camera' => 'startCamera', _ => 'startScreenCapture' }, outParams);
      if (ok == false) {
        streams.remove(key);
        await send(dev, type, {'action': 'stop', 'session': session, 'reason': 'permesso negato sul telefono'});
        return;
      }
      await send(dev, type, {'action': 'offer', 'session': session, 'port': r['port'], 'params': outParams});
      notifyListeners();
      return;
    }
    // The computer offers a stream we consume (desktop / system audio).
    if (action == 'offer' && const {'desktop', 'systemaudio'}.contains(plugin)) {
      final r = await core!.call('streamConnect', {'deviceId': dev, 'port': m['port']});
      final s = StreamSession(plugin, dev, session, r['channelId'] as int);
      streams[key] = s;
      if (plugin == 'desktop') {
        desktopTextureId = await native.invokeMethod<int>('startDecoder', params);
      } else {
        await _nativeSafe('startAudioPlayback', {'sampleRate': params['sampleRate'] ?? 48000, 'channels': params['channels'] ?? 2});
      }
      s.sub = core!.streamFrames(s.channel).listen((f) {
        if (f.kind == frameVideo) native.invokeMethod('decodeFrame', {'data': f.data, 'pts': f.ptsUs, 'flags': f.flags});
        if (f.kind == frameAudio) native.invokeMethod('playPcm', {'data': f.data});
      }, onDone: () => stopStream(plugin, dev, notifyPeer: false));
      notifyListeners();
    }
  }

  /// Ask the computer to start sharing its screen / audio with us.
  Future<void> requestStream(String plugin, String dev, [Map<String, dynamic>? params]) async {
    final session = DateTime.now().microsecondsSinceEpoch.toRadixString(16);
    await send(dev, 'konnekta.$plugin', {'action': 'request', 'session': session, 'params': params ?? {}});
  }

  Future<void> stopStream(String plugin, String dev, {bool notifyPeer = true}) async {
    final s = streams.remove(_skey(plugin, dev));
    if (s == null) return;
    await s.sub?.cancel();
    await core!.call('streamClose', {'channelId': s.channel});
    await _nativeSafe(switch (plugin) {
      'microphone' => 'stopMic',
      'camera' => 'stopCamera',
      'phonescreen' => 'stopScreenCapture',
      'desktop' => 'stopDecoder',
      _ => 'stopAudioPlayback',
    });
    if (plugin == 'desktop') desktopTextureId = null;
    if (notifyPeer) await send(dev, 'konnekta.$plugin', {'action': 'stop', 'session': s.session});
    notifyListeners();
  }

  void _stopStreamsFor(String dev) {
    for (final s in streams.values.where((s) => s.deviceId == dev).toList()) {
      stopStream(s.plugin, dev, notifyPeer: false);
    }
  }

  bool streaming(String plugin, String dev) => streams.containsKey(_skey(plugin, dev));

  // ---------------------------------------------------------- native events --

  void _onNativeEvent(Map<String, dynamic> e) {
    switch (e['type']) {
      case 'notification':
        broadcast('kdeconnect.notification', Map<String, dynamic>.from(e['body'] as Map));
      case 'call':
        broadcast('kdeconnect.telephony', Map<String, dynamic>.from(e['body'] as Map));
      case 'battery':
        broadcast('kdeconnect.battery', {'currentCharge': e['level'], 'isCharging': e['charging'], 'thresholdEvent': e['low'] == true ? 1 : 0});
      case 'dnd':
        if (dndSync) {
          broadcast('konnekta.dnd', {'enabled': e['enabled'], 'changedAt': DateTime.now().millisecondsSinceEpoch, 'userInitiated': true});
        }
      case 'media':
        broadcast('kdeconnect.mpris', Map<String, dynamic>.from(e['body'] as Map));
      case 'clipboard':
        final t = e['text'] as String?;
        if (t != null && t != _lastClipboard) {
          _lastClipboard = t;
          broadcast('kdeconnect.clipboard', {'content': t});
        }
      case 'screenshot':
        if (screenshotAutoTransfer) _sendScreenshot(e['path'] as String, e['sha256'] as String?);
      case 'frame':
        final plugin = e['plugin'] as String;
        for (final s in streams.values.where((s) => s.plugin == plugin)) {
          core!.streamWrite(s.channel, e['kind'] as int, e['flags'] as int? ?? 0, e['pts'] as int? ?? 0, e['data'] as Uint8List);
        }
      case 'error':
        messages.add(e['message'] as String? ?? 'Errore');
    }
  }

  Future<void> _sendScreenshot(String path, String? sha) async {
    for (final d in pairedOnline.where((d) => d.supports('konnekta.screenshot'))) {
      try {
        await core!.call('sendFile', {
          'deviceId': d.id,
          'type': 'konnekta.screenshot',
          'path': path,
          'body': {'filename': path.split('/').last, if (sha != null) 'sha256': sha},
        });
      } catch (e) {
        messages.add('Screenshot non inviato a ${d.name}: $e');
      }
    }
  }

  Future<dynamic> _nativeSafe(String method, [Map<String, dynamic>? args]) async {
    try {
      return await native.invokeMethod(method, args);
    } on MissingPluginException {
      return false;
    } on PlatformException catch (e) {
      messages.add(e.message ?? e.code);
      return false;
    }
  }

  // ------------------------------------------------------------- UI helpers --

  Future<void> sendFiles(String dev, List<String> paths) async {
    for (final p in paths) {
      final f = File(p);
      final stat = await f.stat();
      unawaited(core!
          .call('sendFile', {
            'deviceId': dev,
            'path': p,
            'body': {
              'filename': p.split(Platform.pathSeparator).last,
              'lastModified': stat.modified.millisecondsSinceEpoch,
              'numberOfFiles': paths.length,
            },
          })
          .then((_) => messages.add('Inviato ${p.split('/').last}'))
          .catchError((Object e) => messages.add('Invio non riuscito: $e')));
    }
  }

  Future<void> sendClipboard(String dev) async {
    final data = await Clipboard.getData(Clipboard.kTextPlain);
    if (data?.text != null) await send(dev, 'kdeconnect.clipboard', {'content': data!.text});
  }
}
