import 'package:carbine/multimint.dart';
import 'package:carbine/send.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/utils.dart';
import 'package:flutter/material.dart';

class PaymentPreviewWidget extends StatelessWidget {
  final FederationSelector fed;
  final PaymentPreview paymentPreview;

  const PaymentPreviewWidget({
    super.key,
    required this.fed,
    required this.paymentPreview,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final amount = paymentPreview.amountMsats;
    final amountWithFees = paymentPreview.amountWithFees;
    final fees = amountWithFees - amount;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          'Confirm Lightning Payment',
          style: theme.textTheme.headlineSmall?.copyWith(
            color: theme.colorScheme.primary,
            fontWeight: FontWeight.bold,
          ),
        ),
        const SizedBox(height: 24),
        Container(
          width: double.infinity,
          padding: const EdgeInsets.all(16),
          decoration: BoxDecoration(
            color: theme.colorScheme.surfaceContainer,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(
              color: theme.colorScheme.primary.withOpacity(0.25),
            ),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              buildDetailRow(theme, "Payer Federation", fed.federationName),
              buildDetailRow(theme, 'Amount', formatBalance(amount, true)),
              buildDetailRow(theme, 'Fees', formatBalance(fees, true)),
              buildDetailRow(
                theme,
                'Total',
                formatBalance(amountWithFees, true),
              ),
              buildDetailRow(theme, 'Gateway', paymentPreview.gateway),
              buildDetailRow(theme, 'Payment Hash', paymentPreview.paymentHash),
            ],
          ),
        ),
        const SizedBox(height: 32),
        SizedBox(
          width: double.infinity,
          child: ElevatedButton.icon(
            icon: const Icon(Icons.send, color: Colors.black),
            label: const Text('Send Payment'),
            style: ElevatedButton.styleFrom(
              backgroundColor: theme.colorScheme.primary,
              foregroundColor: Colors.black,
              padding: const EdgeInsets.symmetric(vertical: 16),
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(16),
              ),
            ),
            onPressed: () {
              Navigator.push(
                context,
                MaterialPageRoute(
                  builder:
                      (context) => SendPayment(
                        fed: fed,
                        invoice: paymentPreview.invoice,
                        amountMsats: amount,
                        gateway: paymentPreview.gateway,
                        isLnv2: paymentPreview.isLnv2,
                      ),
                ),
              );
            },
          ),
        ),
        const SizedBox(height: 24), // Padding to prevent tight bottom
      ],
    );
  }
}
