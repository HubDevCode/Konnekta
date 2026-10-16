import 'package:flutter/material.dart';

import 'screens/home.dart';
import 'service.dart';

final service = KonnektaService();

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  service.init();
  runApp(const KonnektaApp());
}

class KonnektaApp extends StatelessWidget {
  const KonnektaApp({super.key});

  @override
  Widget build(BuildContext context) {
    final seed = const Color(0xFF4F46E5);
    return MaterialApp(
      title: 'Konnekta',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(colorSchemeSeed: seed, useMaterial3: true),
      darkTheme: ThemeData(colorSchemeSeed: seed, useMaterial3: true, brightness: Brightness.dark),
      home: const HomeScreen(),
    );
  }
}
