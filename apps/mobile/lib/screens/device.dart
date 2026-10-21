import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';

import '../main.dart';
import '../service.dart';
import 'deck.dart';
import 'desktop.dart';
import 'media.dart';
import 'touchpad.dart';

class DeviceScreen extends StatefulWidget {
  const DeviceScreen({super.key, required this.deviceId});
  final String deviceId;
  @override
  State<DeviceScreen> createState() => _DeviceScreenState();
}

class _DeviceScreenState extends State<DeviceScreen> {
  @override
  void initState() {
    super.initState();
    service.addListener(_changed);
  }

  @override
  void dispose() {
    service.removeListener(_changed);
    super.dispose();
  }

  void _changed() => mounted ? setState(() {}) : null;

  Device? get d => service.devices.where((x) => x.id == widget.deviceId).firstOrNull;

  void _open(Widget w) => Navigator.push(context, MaterialPageRoute(builder: (_) => w));

  @override
  Widget build(BuildContext context) {
    final dev = d;
    if (dev == null) return const Scaffold(body: Center(child: Text('Dispositivo non disponibile')));
    final id = dev.id;
    Widget tile(IconData icon, String label, VoidCallback? onTap, {bool active = false}) => Card(
          color: active ? Theme.of(context).colorScheme.primaryContainer : null,
          child: InkWell(
            borderRadius: BorderRadius.circular(12),
            onTap: dev.connected ? onTap : null,
            child: Column(mainAxisAlignment: MainAxisAlignment.center, children: [
              Icon(icon, size: 30, color: onTap == null || !dev.connected ? Theme.of(context).disabledColor : null),
              const SizedBox(height: 6),
              Text(label, textAlign: TextAlign.center, style: const TextStyle(fontSize: 12)),
            ]),
          ),
        );
    bool cap(String t) => dev.supports(t);
    return Scaffold(
      appBar: AppBar(title: Text(dev.name), actions: [
        IconButton(icon: const Icon(Icons.security), tooltip: 'Permessi', onPressed: () => _open(PermissionsScreen(deviceId: id))),
      ]),
      body: GridView.count(
        padding: const EdgeInsets.all(12),
        crossAxisCount: 3,
        children: [
          tile(Icons.touch_app, 'Touchpad e tastiera', cap('konnekta.input') ? () => _open(TouchpadScreen(deviceId: id)) : null),
          tile(Icons.dashboard_customize, 'Mobile Deck', cap('konnekta.deck.request') ? () => _open(DeckScreen(deviceId: id)) : null),
          tile(Icons.slideshow, 'Presentazione', cap('konnekta.presenter') ? () => _open(PresenterScreen(deviceId: id)) : null),
          tile(Icons.music_note, 'Media', cap('konnekta.media.request') ? () => _open(MediaScreen(deviceId: id)) : null),
          tile(Icons.terminal, 'Comandi', cap('konnekta.commands.request') ? () => _open(CommandsScreen(deviceId: id)) : null),
          tile(Icons.upload_file, 'Invia file', () async {
            final r = await FilePicker.platform.pickFiles(allowMultiple: true);
            final paths = r?.files.map((f) => f.path).whereType<String>().toList() ?? [];
            if (paths.isNotEmpty) await service.sendFiles(id, paths);
          }),
          tile(Icons.content_paste_go, 'Invia appunti', () => service.sendClipboard(id)),
          tile(Icons.notifications_active, 'Ping', () => service.send(id, 'konnekta.ping', {'message': 'Ping dal telefono'})),
          tile(Icons.desktop_windows, 'Desktop remoto', cap('konnekta.desktop') ? () => _open(DesktopScreen(deviceId: id)) : null,
              active: service.streaming('desktop', id)),
          tile(service.streaming('systemaudio', id) ? Icons.volume_up : Icons.speaker, 'Audio del PC',
              cap('konnekta.systemaudio')
                  ? () => service.streaming('systemaudio', id)
                      ? service.stopStream('systemaudio', id)
                      : service.requestStream('systemaudio', id)
                  : null,
              active: service.streaming('systemaudio', id)),
          tile(Icons.mic, service.streaming('microphone', id) ? 'Microfono attivo' : 'Microfono',
              service.streaming('microphone', id) ? () => service.stopStream('microphone', id) : null,
              active: service.streaming('microphone', id)),
          tile(Icons.videocam, service.streaming('camera', id) ? 'Webcam attiva' : 'Webcam',
              service.streaming('camera', id) ? () => service.stopStream('camera', id) : null,
              active: service.streaming('camera', id)),
        ],
      ),
      bottomNavigationBar: const Padding(
        padding: EdgeInsets.all(12),
        child: Text('Microfono, webcam e schermo del telefono si avviano dal computer (Dispositivi › Audio e video). '
            'Puoi interromperli in qualsiasi momento da qui.',
            style: TextStyle(fontSize: 12), textAlign: TextAlign.center),
      ),
    );
  }
}

class PermissionsScreen extends StatefulWidget {
  const PermissionsScreen({super.key, required this.deviceId});
  final String deviceId;
  @override
  State<PermissionsScreen> createState() => _PermissionsScreenState();
}

class _PermissionsScreenState extends State<PermissionsScreen> {
  List<Map<String, dynamic>> perms = [];

  @override
  void initState() {
    super.initState();
    service.api('permissions').then((p) => setState(() => perms = List<Map<String, dynamic>>.from(p as List)));
    service.addListener(_changed);
  }

  @override
  void dispose() {
    service.removeListener(_changed);
    super.dispose();
  }

  void _changed() => mounted ? setState(() {}) : null;

  @override
  Widget build(BuildContext context) {
    final d = service.devices.where((x) => x.id == widget.deviceId).firstOrNull;
    final granted = d?.permissions ?? const [];
    return Scaffold(
      appBar: AppBar(title: const Text('Permessi')),
      body: ListView(children: [
        const Padding(
          padding: EdgeInsets.all(16),
          child: Text('Scegli cosa può fare questo computer con il telefono. Le funzioni sensibili sono disattivate finché non le abiliti.'),
        ),
        for (final p in perms)
          SwitchListTile(
            title: Text(p['label'] as String),
            subtitle: p['sensitive'] == true ? const Text('Sensibile') : null,
            value: granted.contains(p['id']),
            onChanged: (v) async {
              await service.api('setPermission', {'deviceId': widget.deviceId, 'permission': p['id'], 'granted': v});
              await service.refresh();
            },
          ),
        const Divider(),
        ListTile(
          leading: const Icon(Icons.link_off, color: Colors.red),
          title: const Text('Dissocia'),
          onTap: () async {
            await service.api('unpair', {'deviceId': widget.deviceId});
            if (context.mounted) Navigator.popUntil(context, (r) => r.isFirst);
          },
        ),
      ]),
    );
  }
}

class CommandsScreen extends StatefulWidget {
  const CommandsScreen({super.key, required this.deviceId});
  final String deviceId;
  @override
  State<CommandsScreen> createState() => _CommandsScreenState();
}

class _CommandsScreenState extends State<CommandsScreen> {
  @override
  void initState() {
    super.initState();
    service.addListener(_changed);
    service.send(widget.deviceId, 'konnekta.commands.request', {'requestCommandList': true});
  }

  @override
  void dispose() {
    service.removeListener(_changed);
    super.dispose();
  }

  void _changed() => mounted ? setState(() {}) : null;

  @override
  Widget build(BuildContext context) {
    final cmds = service.remoteCommands[widget.deviceId] ?? {};
    return Scaffold(
      appBar: AppBar(title: const Text('Comandi')),
      body: cmds.isEmpty
          ? const Center(child: Text('Nessun comando configurato sul computer.'))
          : ListView(children: [
              for (final e in cmds.entries)
                ListTile(
                  leading: const Icon(Icons.play_circle),
                  title: Text((e.value as Map)['name'] as String? ?? e.key),
                  subtitle: Text((e.value as Map)['command'] as String? ?? ''),
                  onTap: () => service.send(widget.deviceId, 'konnekta.commands.request', {'key': e.key}),
                ),
            ]),
    );
  }
}
