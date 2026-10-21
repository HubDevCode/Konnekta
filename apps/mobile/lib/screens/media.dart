import 'package:flutter/material.dart';

import '../main.dart';

/// Control media playing on the computer.
class MediaScreen extends StatefulWidget {
  const MediaScreen({super.key, required this.deviceId});
  final String deviceId;
  @override
  State<MediaScreen> createState() => _MediaScreenState();
}

class _MediaScreenState extends State<MediaScreen> {
  String? player;

  @override
  void initState() {
    super.initState();
    service.addListener(_changed);
    _req({'requestPlayerList': true, 'requestNowPlaying': true, 'requestVolume': true});
  }

  @override
  void dispose() {
    service.removeListener(_changed);
    super.dispose();
  }

  void _changed() => mounted ? setState(() {}) : null;

  void _req(Map<String, dynamic> b) =>
      service.send(widget.deviceId, 'konnekta.media.request', {if (player != null) 'player': player, ...b});

  @override
  Widget build(BuildContext context) {
    final m = service.pcMedia[widget.deviceId] ?? {};
    final players = service.pcPlayers[widget.deviceId] ?? [];
    final playing = m['isPlaying'] == true;
    return Scaffold(
      appBar: AppBar(title: const Text('Media del computer')),
      body: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(children: [
          if (players.length > 1)
            DropdownButton<String>(
              value: player ?? players.first,
              isExpanded: true,
              items: [for (final p in players) DropdownMenuItem(value: p, child: Text(p))],
              onChanged: (v) {
                setState(() => player = v);
                _req({'requestNowPlaying': true});
              },
            ),
          const Spacer(),
          const Icon(Icons.album, size: 120),
          const SizedBox(height: 16),
          Text(m['title'] as String? ?? 'Niente in riproduzione',
              style: Theme.of(context).textTheme.titleLarge, textAlign: TextAlign.center),
          Text([m['artist'], m['album']].whereType<String>().where((s) => s.isNotEmpty).join(' — '), textAlign: TextAlign.center),
          const Spacer(),
          Row(mainAxisAlignment: MainAxisAlignment.center, children: [
            IconButton(iconSize: 48, icon: const Icon(Icons.skip_previous), onPressed: () => _req({'action': 'Previous'})),
            IconButton.filled(
                iconSize: 64, icon: Icon(playing ? Icons.pause : Icons.play_arrow), onPressed: () => _req({'action': 'PlayPause'})),
            IconButton(iconSize: 48, icon: const Icon(Icons.skip_next), onPressed: () => _req({'action': 'Next'})),
          ]),
          Row(children: [
            const Icon(Icons.volume_down),
            Expanded(
              child: Slider(
                value: ((m['volume'] as num?) ?? 50).toDouble().clamp(0, 100),
                max: 100,
                onChanged: (v) => setState(() => service.pcMedia[widget.deviceId] = {...m, 'volume': v.round()}),
                onChangeEnd: (v) => _req({'setVolume': v.round()}),
              ),
            ),
            const Icon(Icons.volume_up),
          ]),
        ]),
      ),
    );
  }
}
