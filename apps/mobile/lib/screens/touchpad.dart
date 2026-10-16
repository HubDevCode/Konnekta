import 'package:flutter/material.dart';

import '../main.dart';

/// Touchpad + keyboard (KDE Connect mousepad protocol).
class TouchpadScreen extends StatefulWidget {
  const TouchpadScreen({super.key, required this.deviceId});
  final String deviceId;
  @override
  State<TouchpadScreen> createState() => _TouchpadScreenState();
}

class _TouchpadScreenState extends State<TouchpadScreen> {
  final _text = TextEditingController();
  final _focus = FocusNode();
  double sensitivity = 1.6;
  int _pointers = 0;
  bool ctrl = false, alt = false, shift = false, meta = false;
  String _last = '';
  Offset _dragOrigin = Offset.zero;

  void _send(Map<String, dynamic> b) => service.send(widget.deviceId, 'kdeconnect.mousepad.request', b);

  Map<String, dynamic> get _mods => {
        if (ctrl) 'ctrl': true,
        if (alt) 'alt': true,
        if (shift) 'shift': true,
        if (meta) 'super': true,
      };

  void _special(int code) => _send({'specialKey': code, ..._mods});

  void _onText(String v) {
    // Diff against the previous value: send typed chars or backspaces.
    if (v.length > _last.length && v.startsWith(_last)) {
      _send({'key': v.substring(_last.length), ..._mods});
    } else if (v.length < _last.length) {
      for (var i = 0; i < _last.length - v.length; i++) {
        _special(1);
      }
    }
    _last = v;
    if (v.length > 64) {
      _text.clear();
      _last = '';
    }
  }

  @override
  Widget build(BuildContext context) {
    Widget mod(String l, bool v, void Function(bool) set) => FilterChip(label: Text(l), selected: v, onSelected: (x) => setState(() => set(x)));
    Widget key(String l, int code) => ActionChip(label: Text(l), onPressed: () => _special(code));
    return Scaffold(
      appBar: AppBar(title: const Text('Touchpad'), actions: [
        IconButton(icon: const Icon(Icons.keyboard), onPressed: () => _focus.requestFocus()),
      ]),
      body: Column(children: [
        Expanded(
          child: Listener(
            onPointerDown: (_) => _pointers++,
            onPointerUp: (_) => _pointers = (_pointers - 1).clamp(0, 10),
            onPointerCancel: (_) => _pointers = 0,
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: () => _send({'singleclick': true}),
              onDoubleTap: () => _send({'doubleclick': true}),
              onLongPressStart: (_) {
                _dragOrigin = Offset.zero;
                _send({'singlehold': true});
              },
              onLongPressMoveUpdate: (d) {
                final delta = d.offsetFromOrigin - _dragOrigin;
                _dragOrigin = d.offsetFromOrigin;
                _send({'dx': delta.dx * sensitivity, 'dy': delta.dy * sensitivity});
              },
              onLongPressEnd: (_) => _send({'singlerelease': true}),
              onSecondaryTap: () => _send({'rightclick': true}),
              onScaleUpdate: (d) {
                final delta = d.focalPointDelta;
                if (_pointers >= 2) {
                  _send({'dx': delta.dx, 'dy': delta.dy, 'scroll': true});
                } else {
                  _send({'dx': delta.dx * sensitivity, 'dy': delta.dy * sensitivity});
                }
              },
              child: Container(
                margin: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(16),
                  color: Theme.of(context).colorScheme.surfaceContainerHighest,
                ),
                alignment: Alignment.center,
                child: const Text('Tocca per clic · due dita per scorrere · tieni premuto per trascinare',
                    textAlign: TextAlign.center),
              ),
            ),
          ),
        ),
        Row(children: [
          Expanded(child: TextButton(onPressed: () => _send({'singleclick': true}), child: const Text('Sinistro'))),
          Expanded(child: TextButton(onPressed: () => _send({'middleclick': true}), child: const Text('Centrale'))),
          Expanded(child: TextButton(onPressed: () => _send({'rightclick': true}), child: const Text('Destro'))),
        ]),
        SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: Wrap(spacing: 6, children: [
            mod('Ctrl', ctrl, (v) => ctrl = v),
            mod('Alt', alt, (v) => alt = v),
            mod('Shift', shift, (v) => shift = v),
            mod('Super', meta, (v) => meta = v),
            key('Esc', 14), key('Tab', 2), key('Invio', 12), key('Canc', 13),
            key('←', 4), key('↑', 5), key('→', 6), key('↓', 7),
            key('Home', 10), key('Fine', 11), key('PgSu', 8), key('PgGiù', 9),
            for (var i = 1; i <= 12; i++) key('F$i', 20 + i),
          ]),
        ),
        Padding(
          padding: const EdgeInsets.all(8),
          child: TextField(
            controller: _text,
            focusNode: _focus,
            autocorrect: false,
            enableSuggestions: false,
            decoration: const InputDecoration(hintText: 'Scrivi sul computer…', border: OutlineInputBorder()),
            onChanged: _onText,
            onSubmitted: (_) {
              _special(12);
              _text.clear();
              _last = '';
              _focus.requestFocus();
            },
          ),
        ),
      ]),
    );
  }
}

/// Presentation remote: named slide commands (Konnekta) + laser pointer.
class PresenterScreen extends StatelessWidget {
  const PresenterScreen({super.key, required this.deviceId});
  final String deviceId;

  void _cmd(String c) => service.send(deviceId, 'kdeconnect.presenter', {'konnektaKey': c});

  @override
  Widget build(BuildContext context) {
    Widget big(IconData i, String label, String cmd) => Expanded(
          child: Padding(
            padding: const EdgeInsets.all(6),
            child: FilledButton.tonal(
              style: FilledButton.styleFrom(shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(18))),
              onPressed: () => _cmd(cmd),
              child: Column(mainAxisAlignment: MainAxisAlignment.center, children: [Icon(i, size: 48), Text(label)]),
            ),
          ),
        );
    return Scaffold(
      appBar: AppBar(title: const Text('Presentazione')),
      body: Column(children: [
        Expanded(flex: 3, child: Row(children: [big(Icons.chevron_left, 'Indietro', 'previous'), big(Icons.chevron_right, 'Avanti', 'next')])),
        Expanded(
          child: Row(children: [
            big(Icons.play_arrow, 'Avvia', 'start'),
            big(Icons.stop, 'Esci', 'stop'),
            big(Icons.dark_mode, 'Nero', 'black'),
          ]),
        ),
        Expanded(
          flex: 2,
          child: GestureDetector(
            onPanUpdate: (d) => service.send(deviceId, 'kdeconnect.presenter', {'dx': d.delta.dx / 1000, 'dy': d.delta.dy / 1000}),
            onPanEnd: (_) => service.send(deviceId, 'kdeconnect.presenter', {'stop': true}),
            child: Container(
              margin: const EdgeInsets.all(12),
              decoration: BoxDecoration(color: Colors.red.withValues(alpha: .12), borderRadius: BorderRadius.circular(16)),
              alignment: Alignment.center,
              child: const Text('Trascina qui per muovere il puntatore'),
            ),
          ),
        ),
      ]),
    );
  }
}
