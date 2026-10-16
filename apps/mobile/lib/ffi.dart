// Bindings to the Rust core (crates/konnekta-ffi). Blocking calls run on
// background isolates so the UI never waits on the network.

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

typedef _StartC = Pointer<Void> Function(Pointer<Utf8>, Pointer<Utf8>);
typedef _CallC = Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>);
typedef _NextC = Pointer<Utf8> Function(Pointer<Void>, Int32);
typedef _NextD = Pointer<Utf8> Function(Pointer<Void>, int);
typedef _FreeC = Void Function(Pointer<Utf8>);
typedef _FreeD = void Function(Pointer<Utf8>);
typedef _StopC = Void Function(Pointer<Void>);
typedef _StopD = void Function(Pointer<Void>);
typedef _WriteC = Int32 Function(Pointer<Void>, Uint64, Uint8, Uint8, Uint64, Pointer<Uint8>, Size);
typedef _WriteD = int Function(Pointer<Void>, int, int, int, int, Pointer<Uint8>, int);
typedef _ReadC = Int64 Function(Pointer<Void>, Uint64, Pointer<Uint8>, Size, Pointer<Uint8>, Pointer<Uint64>, Int32);
typedef _ReadD = int Function(Pointer<Void>, int, Pointer<Uint8>, int, Pointer<Uint8>, Pointer<Uint64>, int);

DynamicLibrary _open() {
  if (Platform.isAndroid || Platform.isLinux) return DynamicLibrary.open('libkonnekta_ffi.so');
  if (Platform.isIOS || Platform.isMacOS) return DynamicLibrary.process();
  if (Platform.isWindows) return DynamicLibrary.open('konnekta_ffi.dll');
  throw UnsupportedError('platform not supported');
}

class _Lib {
  _Lib(DynamicLibrary l)
      : start = l.lookupFunction<_StartC, _StartC>('konnekta_start'),
        call = l.lookupFunction<_CallC, _CallC>('konnekta_call'),
        next = l.lookupFunction<_NextC, _NextD>('konnekta_next_event'),
        free = l.lookupFunction<_FreeC, _FreeD>('konnekta_free_string'),
        stop = l.lookupFunction<_StopC, _StopD>('konnekta_stop'),
        write = l.lookupFunction<_WriteC, _WriteD>('konnekta_stream_write'),
        read = l.lookupFunction<_ReadC, _ReadD>('konnekta_stream_read');
  final _StartC start;
  final _CallC call;
  final _NextD next;
  final _FreeD free;
  final _StopD stop;
  final _WriteD write;
  final _ReadD read;
}

final _Lib _lib = _Lib(_open());

String _take(Pointer<Utf8> p) {
  final s = p.toDartString();
  _lib.free(p);
  return s;
}

class KonnektaException implements Exception {
  KonnektaException(this.message);
  final String message;
  @override
  String toString() => message;
}

/// Frame kinds / flags (see konnekta.h).
const frameAudio = 1, frameVideo = 2, frameControl = 3;
const flagKeyframe = 1, flagConfig = 2;

class Frame {
  Frame(this.kind, this.flags, this.ptsUs, this.data);
  final int kind, flags, ptsUs;
  final Uint8List data;
}

class KonnektaCore {
  KonnektaCore._(this._addr);
  final int _addr;

  Pointer<Void> get _h => Pointer<Void>.fromAddress(_addr);

  static Future<KonnektaCore> start(Map<String, dynamic> config, String dataDir) async {
    final cfg = jsonEncode(config);
    final addr = await Isolate.run(() {
      final c = cfg.toNativeUtf8();
      final d = dataDir.toNativeUtf8();
      final h = _lib.start(c, d);
      malloc.free(c);
      malloc.free(d);
      return h.address;
    });
    if (addr == 0) throw KonnektaException('Impossibile avviare il core Konnekta');
    return KonnektaCore._(addr);
  }

  static dynamic _callSync(int addr, String method, String params) {
    final m = method.toNativeUtf8();
    final p = params.toNativeUtf8();
    final r = _take(_lib.call(Pointer<Void>.fromAddress(addr), m, p));
    malloc.free(m);
    malloc.free(p);
    final v = jsonDecode(r) as Map<String, dynamic>;
    if (v['error'] != null) throw KonnektaException(v['error'] as String);
    return v['result'];
  }

  Future<dynamic> call(String method, [Map<String, dynamic>? params]) {
    final addr = _addr;
    final p = jsonEncode(params ?? {});
    return Isolate.run(() => _callSync(addr, method, p));
  }

  /// Send a packet for a bridge plugin (permission-checked by the core).
  Future<void> sendPacket(String deviceId, String type, Map<String, dynamic> body) =>
      call('sendPacket', {'deviceId': deviceId, 'type': type, 'body': body});

  /// Event stream from a dedicated isolate.
  Stream<Map<String, dynamic>> events() async* {
    final port = ReceivePort();
    await Isolate.spawn(_eventLoop, [_addr, port.sendPort]);
    await for (final msg in port) {
      yield jsonDecode(msg as String) as Map<String, dynamic>;
    }
  }

  static void _eventLoop(List<dynamic> args) {
    final h = Pointer<Void>.fromAddress(args[0] as int);
    final out = args[1] as SendPort;
    while (true) {
      final p = _lib.next(h, 1000);
      if (p.address != 0) out.send(_take(p));
    }
  }

  /// Queue a media frame on a producer channel (cheap, non-blocking).
  /// Returns 0 queued, 1 dropped, -1 closed.
  int streamWrite(int channel, int kind, int flags, int ptsUs, Uint8List data) {
    final buf = malloc<Uint8>(data.isEmpty ? 1 : data.length);
    buf.asTypedList(data.length).setAll(0, data);
    final r = _lib.write(_h, channel, kind, flags, ptsUs, buf, data.length);
    malloc.free(buf);
    return r;
  }

  /// Read frames from a consumer channel on a background isolate.
  Stream<Frame> streamFrames(int channel) async* {
    final port = ReceivePort();
    await Isolate.spawn(_readLoop, [_addr, channel, port.sendPort]);
    await for (final msg in port) {
      if (msg == null) break;
      final m = msg as List;
      yield Frame(m[0] as int, m[1] as int, m[2] as int, (m[3] as TransferableTypedData).materialize().asUint8List());
    }
  }

  static void _readLoop(List<dynamic> args) {
    final h = Pointer<Void>.fromAddress(args[0] as int);
    final ch = args[1] as int;
    final out = args[2] as SendPort;
    const cap = 8 * 1024 * 1024;
    final buf = malloc<Uint8>(cap);
    final meta = malloc<Uint8>(2);
    final pts = malloc<Uint64>(1);
    while (true) {
      final n = _lib.read(h, ch, buf, cap, meta, pts, 500);
      if (n == -1) break;
      if (n <= 0) continue;
      final data = Uint8List.fromList(buf.asTypedList(n));
      out.send([meta[0], meta[1], pts.value, TransferableTypedData.fromList([data])]);
    }
    malloc.free(buf);
    malloc.free(meta);
    malloc.free(pts);
    out.send(null);
  }

  void stop() => _lib.stop(_h);
}
