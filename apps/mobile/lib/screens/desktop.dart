import 'package:flutter/material.dart';

import '../main.dart';

/// Remote desktop: the computer streams H.264 which the native decoder renders
/// into a Flutter texture; touches become absolute pointer events.
class DesktopScreen extends StatefulWidget {
  const DesktopScreen({super.key, required this.deviceId});
  final String deviceId;
  @override
  State<DesktopScreen> createState() => _DesktopScreenState();
}

class _DesktopScreenState extends State<DesktopScreen> {
  double aspect = 16 / 9;

  @override
  void initState() {
    super.initState();
    service.addListener(_changed);
    if (!service.streaming('desktop', widget.deviceId)) {
      final size = WidgetsBinding.instance.platformDispatcher.views.first.physicalSize;
      service.requestStream('desktop', widget.deviceId, {'height': size.shortestSide.round().clamp(480, 1080)});
    }
  }

  @override
  void dispose() {
    service.removeListener(_changed);
    service.stopStream('desktop', widget.deviceId);
    super.dispose();
  }

  void _changed() => mounted ? setState(() {}) : null;

  void _pointer(Offset local, Size box, Map<String, dynamic> extra) {
    service.send(widget.deviceId, 'konnekta.input', {
      'konnektaAbsX': (local.dx / box.width).clamp(0.0, 1.0),
      'konnektaAbsY': (local.dy / box.height).clamp(0.0, 1.0),
      ...extra,
    });
  }

  @override
  Widget build(BuildContext context) {
    final tex = service.desktopTextureId;
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(title: const Text('Desktop remoto'), actions: [
        IconButton(icon: const Icon(Icons.stop_circle), onPressed: () => Navigator.pop(context)),
      ]),
      body: Center(
        child: tex == null
            ? const Column(mainAxisSize: MainAxisSize.min, children: [
                CircularProgressIndicator(),
                SizedBox(height: 12),
                Text('In attesa del computer…', style: TextStyle(color: Colors.white70)),
                Text('Serve il permesso "Remote desktop" sul computer.', style: TextStyle(color: Colors.white38, fontSize: 12)),
              ])
            : AspectRatio(
                aspectRatio: aspect,
                child: LayoutBuilder(builder: (context, c) {
                  final box = Size(c.maxWidth, c.maxHeight);
                  return GestureDetector(
                    onTapUp: (d) => _pointer(d.localPosition, box, {'singleclick': true}),
                    onLongPressStart: (d) => _pointer(d.localPosition, box, {'rightclick': true}),
                    onPanStart: (d) => _pointer(d.localPosition, box, {'singlehold': true}),
                    onPanUpdate: (d) => _pointer(d.localPosition, box, {}),
                    onPanEnd: (_) => service.send(widget.deviceId, 'konnekta.input', {'singlerelease': true}),
                    child: Texture(textureId: tex),
                  );
                }),
              ),
      ),
    );
  }
}
