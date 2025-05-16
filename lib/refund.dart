import 'package:carbine/success.dart';
import 'package:flutter/material.dart';
import 'package:carbine/lib.dart';

/// A simple confirmation page for refunding the full on-chain balance
class RefundConfirmationPage extends StatelessWidget {
  final FederationSelector fed;
  final BigInt balanceMsats;

  // Static refund address
  static const String refundAddress =
      'tb1qd28npep0s8frcm3y7dxqajkcy2m40eysplyr9v';

  const RefundConfirmationPage({
    super.key,
    required this.fed,
    required this.balanceMsats,
  });

  @override
  Widget build(BuildContext context) {
    // Convert msats to sats
    final sats = balanceMsats ~/ BigInt.from(1000);

    return Scaffold(
      appBar: AppBar(title: const Text('Confirm Refund'), centerTitle: true),
      body: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              'You are about to send your full balance of $sats sats to:',
              textAlign: TextAlign.center,
              style: Theme.of(context).textTheme.bodyLarge,
            ),
            const SizedBox(height: 12),
            SelectableText(
              refundAddress,
              textAlign: TextAlign.center,
              style: Theme.of(
                context,
              ).textTheme.bodyMedium?.copyWith(fontWeight: FontWeight.w600),
            ),
            const SizedBox(height: 24),
            ElevatedButton(
              onPressed: () async {
                final navigator = Navigator.of(context);
                final rootNav = Navigator.of(context, rootNavigator: true);

                await refund(federationId: fed.federationId);

                navigator.push(
                  MaterialPageRoute(
                    builder:
                        (context) => Success(
                          lightning: true,
                          received: false,
                          amountMsats: balanceMsats,
                        ),
                  ),
                );

                await Future.delayed(const Duration(seconds: 4));
                rootNav.popUntil((route) => route.isFirst);
              },
              child: const Text('Confirm'),
            ),
            const SizedBox(height: 12),
            TextButton(
              onPressed: () {
                Navigator.of(context).pop(false);
              },
              child: const Text('Cancel'),
            ),
          ],
        ),
      ),
    );
  }
}
