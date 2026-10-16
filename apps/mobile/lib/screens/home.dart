import 'dart:async';

import 'package:flutter/material.dart';

import '../main.dart';
import '../service.dart';
import 'device.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});
  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  StreamSubscription<String>? _msgs;
  PairPrompt? _shown;

  @override
  void initState() {
    super.initState();
    service.addListener(_changed);
    _msgs = service.messages.stream.listen((m) {
      if (mounted) ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(m)));
    });
  }

  @override
  void dispose() {
    service.removeListener(_changed);
    _msgs?.cancel();
    super.dispose();
  }

  void _changed() {
    if (!mounted) return;
    setState(() {});
    final p = service.pairPrompt;
    if (p != null && p != _shown) {
      _shown = p;
      _showPair(p);
    }
  }

  void _showPair(PairPrompt p) {
    showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (ctx) => AlertDialog(
        title: Text(p.outgoing ? 'Verifica il codice' : 'Richiesta di associazione'),
        content: Column(mainAxisSize: MainAxisSize.min, children: [
          Text(p.outgoing
              ? 'Conferma sul computer solo se mostra questo codice:'
              : '"${p.deviceName}" vuole associarsi. Accetta solo se il computer mostra lo stesso codice:'),
          const SizedBox(height: 16),
          SelectableText(p.code,
              style: const TextStyle(fontSize: 32, fontWeight: FontWeight.bold, letterSpacing: 4, fontFamily: 'monospace')),
        ]),
        actions: p.outgoing
            ? [TextButton(onPressed: () => Navigator.pop(ctx), child: const Text('Chiudi'))]
            : [
                TextButton(
                    onPressed: () {
                      service.api('rejectPair', {'deviceId': p.deviceId});
                      Navigator.pop(ctx);
                    },
                    child: const Text('Rifiuta')),
                FilledButton(
                    onPressed: () {
                      service.api('acceptPair', {'deviceId': p.deviceId});
                      Navigator.pop(ctx);
                    },
                    child: const Text('Accetta')),
              ],
      ),
    );
  }

  Future<void> _connectByIp() async {
    final ctl = TextEditingController();
    final ip = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Aggiungi dispositivo per IP'),
        content: TextField(controller: ctl, keyboardType: TextInputType.url, decoration: const InputDecoration(hintText: '192.168.1.10')),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx), child: const Text('Annulla')),
          FilledButton(onPressed: () => Navigator.pop(ctx, ctl.text.trim()), child: const Text('Connetti')),
        ],
      ),
    );
    if (ip != null && ip.isNotEmpty) await service.api('connect', {'address': ip});
  }

  IconData _icon(String t) => switch (t) {
        'laptop' => Icons.laptop,
        'phone' => Icons.smartphone,
        'tablet' => Icons.tablet,
        'tv' => Icons.tv,
        _ => Icons.desktop_windows,
      };

  @override
  Widget build(BuildContext context) {
    if (service.error != null) {
      return Scaffold(body: Center(child: Padding(padding: const EdgeInsets.all(24), child: Text(service.error!))));
    }
    if (service.core == null) return const Scaffold(body: Center(child: CircularProgressIndicator()));
    final devices = service.devices;
    return Scaffold(
      appBar: AppBar(title: const Text('Konnekta'), actions: [
        IconButton(icon: const Icon(Icons.refresh), onPressed: () => service.api('refresh').then((_) => service.refresh())),
        IconButton(icon: const Icon(Icons.add_link), tooltip: 'Connetti per IP', onPressed: _connectByIp),
      ]),
      body: RefreshIndicator(
        onRefresh: () => service.api('refresh').then((_) => service.refresh()),
        child: ListView(children: [
          if (devices.isEmpty)
            const Padding(
              padding: EdgeInsets.all(32),
              child: Text('Nessun computer trovato. Avvia Konnekta (o KDE Connect) sul computer nella stessa rete Wi-Fi.',
                  textAlign: TextAlign.center),
            ),
          for (final d in devices)
            ListTile(
              leading: Icon(_icon(d.type), size: 32),
              title: Text(d.name),
              subtitle: Text(d.paired ? (d.connected ? 'Associato · connesso' : 'Associato · offline') : 'Non associato'),
              trailing: d.paired
                  ? Icon(Icons.circle, size: 12, color: d.connected ? Colors.green : Colors.grey)
                  : d.connected
                      ? FilledButton.tonal(onPressed: () => service.api('pair', {'deviceId': d.id}), child: const Text('Associa'))
                      : null,
              onTap: d.paired ? () => Navigator.push(context, MaterialPageRoute(builder: (_) => DeviceScreen(deviceId: d.id))) : null,
            ),
          const Divider(),
          ListTile(
            leading: const Icon(Icons.fingerprint),
            title: const Text('Questo telefono'),
            subtitle: Text('${service.me['deviceName'] ?? ''}\n${service.me['fingerprint'] ?? ''}',
                style: const TextStyle(fontSize: 11)),
            isThreeLine: true,
          ),
        ]),
      ),
    );
  }
}
