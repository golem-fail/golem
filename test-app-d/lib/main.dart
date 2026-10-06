import 'package:flutter/material.dart';

void main() {
  runApp(const CounterApp());
}

class CounterApp extends StatelessWidget {
  const CounterApp({super.key});

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(title: 'Test Flutter (Golem)', home: CounterScreen());
  }
}

class CounterScreen extends StatefulWidget {
  const CounterScreen({super.key});

  @override
  State<CounterScreen> createState() => _CounterScreenState();
}

class _CounterScreenState extends State<CounterScreen> {
  int _count = 0;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            spacing: 12,
            children: [
              // Semantics(identifier:) becomes the iOS accessibilityIdentifier
              // and the Android resource-id, which golem reads as an id.
              Semantics(
                identifier: 'counter-title',
                child: Text(
                  'Flutter Counter',
                  style: Theme.of(context).textTheme.headlineMedium,
                ),
              ),
              Text('$_count'),
              Row(
                spacing: 12,
                children: [
                  ElevatedButton(
                    onPressed: () => setState(() => _count++),
                    child: const Text('+'),
                  ),
                  ElevatedButton(
                    onPressed: () => setState(() => _count--),
                    child: const Text('-'),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}
