import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../main.dart';

/// Stream Deck-like grid. Layout comes from the computer; pressing a button
/// only sends its id.
class DeckScreen extends StatefulWidget {
  const DeckScreen({super.key, required this.deviceId});
  final String deviceId;
  @override
  State<DeckScreen> createState() => _DeckScreenState();
}

const _icons = <String, IconData>{
  'play_pause': Icons.play_arrow, 'skip_previous': Icons.skip_previous, 'skip_next': Icons.skip_next,
  'volume': Icons.volume_up, 'volume_off': Icons.volume_off, 'dnd': Icons.do_not_disturb_on,
  'content_copy': Icons.content_copy, 'content_paste': Icons.content_paste, 'search': Icons.search, 'tab': Icons.tab,
  'looks_one': Icons.looks_one, 'looks_two': Icons.looks_two, 'looks_3': Icons.looks_3, 'sensors': Icons.sensors,
  'fiber_manual_record': Icons.fiber_manual_record, 'mic_off': Icons.mic_off, 'arrow_back': Icons.arrow_back,
  'arrow_forward': Icons.arrow_forward, 'play_arrow': Icons.play_arrow, 'dark_mode': Icons.dark_mode, 'stop': Icons.stop,
  'screenshot': Icons.screenshot, 'headset_off': Icons.headset_off, 'record_voice_over': Icons.record_voice_over,
  'work': Icons.work, 'videocam': Icons.videocam, 'slideshow': Icons.slideshow, 'sports_esports': Icons.sports_esports,
};

Color _color(String? hex, Color fallback) {
  if (hex == null || !hex.startsWith('#') || hex.length != 7) return fallback;
  return Color(int.parse('FF${hex.substring(1)}', radix: 16));
}

class _DeckScreenState extends State<DeckScreen> {
  String? profileId;
  int page = 0;
  String category = '';

  @override
  void initState() {
    super.initState();
    service.addListener(_changed);
    service.send(widget.deviceId, 'konnekta.deck.request', {'request': 'layout'});
  }

  @override
  void dispose() {
    service.removeListener(_changed);
    super.dispose();
  }

  void _changed() => mounted ? setState(() {}) : null;

  @override
  Widget build(BuildContext context) {
    final layout = service.deckLayout[widget.deviceId];
    if (layout == null) {
      return Scaffold(appBar: AppBar(title: const Text('Mobile Deck')), body: const Center(child: CircularProgressIndicator()));
    }
    final profiles = List<Map<String, dynamic>>.from(layout['profiles'] as List);
    final active = profileId ?? layout['activeProfile'] as String?;
    final prof = profiles.firstWhere((p) => p['id'] == active, orElse: () => profiles.first);
    final pages = List<Map<String, dynamic>>.from(prof['pages'] as List);
    if (page >= pages.length) page = 0;
    final pg = pages.isEmpty ? null : pages[page];
    final cols = (prof['columns'] as num).toInt();
    final rows = (prof['rows'] as num).toInt();
    final buttons = {for (final b in List<Map<String, dynamic>>.from(pg?['buttons'] as List? ?? [])) (b['position'] as num).toInt(): b};
    final cats = buttons.values.map((b) => b['category'] as String? ?? '').where((c) => c.isNotEmpty).toSet().toList();
    final states = service.deckStates[widget.deviceId] ?? {};
    final scheme = Theme.of(context).colorScheme;

    return Scaffold(
      appBar: AppBar(
        title: DropdownButton<String>(
          value: prof['id'] as String,
          underline: const SizedBox(),
          items: [for (final p in profiles) DropdownMenuItem(value: p['id'] as String, child: Text(p['name'] as String))],
          onChanged: (v) {
            setState(() {
              profileId = v;
              page = 0;
            });
            service.send(widget.deviceId, 'konnekta.deck.request', {'request': 'setProfile', 'profile': v});
          },
        ),
        bottom: pages.length > 1
            ? PreferredSize(
                preferredSize: const Size.fromHeight(44),
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(children: [
                    for (var i = 0; i < pages.length; i++)
                      Padding(
                        padding: const EdgeInsets.symmetric(horizontal: 4),
                        child: ChoiceChip(label: Text(pages[i]['name'] as String), selected: i == page, onSelected: (_) => setState(() => page = i)),
                      ),
                  ]),
                ),
              )
            : null,
      ),
      body: Column(children: [
        if (cats.length > 1)
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: Row(children: [
              ChoiceChip(label: const Text('Tutti'), selected: category.isEmpty, onSelected: (_) => setState(() => category = '')),
              for (final c in cats)
                Padding(
                  padding: const EdgeInsets.only(left: 6),
                  child: ChoiceChip(label: Text(c), selected: category == c, onSelected: (_) => setState(() => category = c)),
                ),
            ]),
          ),
        Expanded(
          child: GridView.count(
            padding: const EdgeInsets.all(10),
            crossAxisCount: cols,
            mainAxisSpacing: 10,
            crossAxisSpacing: 10,
            children: [
              for (var pos = 0; pos < cols * rows; pos++)
                if (buttons[pos] case final b?)
                  Opacity(
                    opacity: category.isEmpty || b['category'] == category ? 1 : .25,
                    child: _DeckButton(
                      b: b,
                      state: states[b['id']] as Map?,
                      fallback: scheme.primary,
                      onPressed: () {
                        HapticFeedback.lightImpact();
                        service.send(widget.deviceId, 'konnekta.deck.request',
                            {'request': 'press', 'profile': prof['id'], 'page': pg!['id'], 'button': b['id']});
                      },
                    ),
                  )
                else
                  const SizedBox(),
            ],
          ),
        ),
      ]),
    );
  }
}

class _DeckButton extends StatelessWidget {
  const _DeckButton({required this.b, required this.state, required this.fallback, required this.onPressed});
  final Map<String, dynamic> b;
  final Map? state;
  final Color fallback;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final active = state?['active'] == true;
    final color = _color(active ? (b['activeColor'] as String? ?? b['color'] as String?) : b['color'] as String?, fallback);
    final icon = _icons[b['icon']];
    final label = (state?['label'] as String?) ?? b['label'] as String;
    return Material(
      color: color,
      borderRadius: BorderRadius.circular(16),
      elevation: active ? 6 : 1,
      child: InkWell(
        borderRadius: BorderRadius.circular(16),
        onTap: onPressed,
        child: Padding(
          padding: const EdgeInsets.all(6),
          child: Column(mainAxisAlignment: MainAxisAlignment.center, children: [
            if (icon != null) Icon(icon, color: Colors.white, size: 30) else Text(b['icon'] as String? ?? '', style: const TextStyle(fontSize: 24)),
            const SizedBox(height: 4),
            Text(label, maxLines: 2, overflow: TextOverflow.ellipsis, textAlign: TextAlign.center,
                style: const TextStyle(color: Colors.white, fontWeight: FontWeight.w600, fontSize: 12)),
          ]),
        ),
      ),
    );
  }
}
