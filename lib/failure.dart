import 'package:carbine/utils.dart';
import 'package:flutter/material.dart';
import 'package:flutter_animate/flutter_animate.dart';

class Failure extends StatelessWidget {
  final BigInt amountMsats;

  const Failure({super.key, required this.amountMsats});

  @override
  Widget build(BuildContext context) {
    final displayAmount = formatBalance(amountMsats, false);

    return Scaffold(
      body: Stack(
        alignment: Alignment.center,
        children: [
          Center(
            child: Animate(
              effects: [
                ScaleEffect(duration: 600.ms, curve: Curves.easeOutBack),
                FadeEffect(duration: 600.ms, curve: Curves.easeIn),
              ],
              child: Column(
                mainAxisSize: MainAxisSize.min,
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  // Red Circle with X Icon
                  Container(
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      color: Colors.redAccent.withOpacity(0.8),
                    ),
                    padding: const EdgeInsets.all(24),
                    child: const Icon(
                      Icons.close,
                      size: 64,
                      color: Colors.white,
                    ),
                  ),
                  const SizedBox(height: 24),

                  // Failure message
                  Text(
                    'Failed to send $displayAmount',
                    style: const TextStyle(
                      fontSize: 28,
                      fontWeight: FontWeight.bold,
                    ),
                    textAlign: TextAlign.center,
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
