import 'package:ecashapp/app.dart';
import 'package:ecashapp/detail_row.dart';
import 'package:ecashapp/lib.dart';
import 'package:ecashapp/multimint.dart';
import 'package:ecashapp/redeem_ecash.dart';
import 'package:ecashapp/theme.dart';
import 'package:ecashapp/toast.dart';
import 'package:ecashapp/utils.dart';
import 'package:flutter/material.dart';

class TransactionDetails extends StatefulWidget {
  final Transaction tx;
  final Icon icon;
  final Map<String, String> details;
  final FederationSelector fed;

  const TransactionDetails({
    super.key,
    required this.tx,
    required this.icon,
    required this.details,
    required this.fed,
  });

  @override
  State<TransactionDetails> createState() => _TransactionDetailsState();
}

class _TransactionDetailsState extends State<TransactionDetails> {
  bool _checking = false;

  String _getTitleFromKind() {
    switch (widget.tx.kind) {
      case TransactionKind_LightningReceive():
        return "Lightning Receive";
      case TransactionKind_LightningSend():
        return "Lightning Send";
      case TransactionKind_LightningRecurring():
        return "Lightning Address Receive";
      case TransactionKind_EcashReceive():
        return "Ecash Receive";
      case TransactionKind_EcashSend():
        return "Ecash Send";
      case TransactionKind_OnchainReceive():
        return "Onchain Receive";
      case TransactionKind_OnchainSend():
        return "Onchain Send";
    }
  }

  Future<void> _checkClaimStatus() async {
    setState(() {
      _checking = true;
    });

    try {
      final ecash = widget.details["Ecash"];
      if (ecash != null) {
        final result = await checkEcashSpent(
          federationId: widget.fed.federationId,
          ecash: ecash,
        );
        if (result) {
          ToastService().show(message: "This ecash has been claimed", duration: const Duration(seconds: 5), onTap: () {}, icon: Icon(Icons.info));
        } else {
          ToastService().show(message: "This ecash has not been claimed yet", duration: const Duration(seconds: 5), onTap: () {}, icon: Icon(Icons.info));
        }
      }
    } catch (e) {
      AppLogger.instance.error("Error checking claim status: $e");
      ToastService().show(message: "Unable to check ecash status", duration: const Duration(seconds: 5), onTap: () {}, icon: Icon(Icons.error));
    } finally {
      setState(() {
        _checking = false;
      });
    }
  }

  Future<void> _redeemEcash() async {
    Navigator.of(context).pop(); // dismiss current modal
    await Future.delayed(Duration.zero); // wait for pop to complete

    final ecash = widget.details["Ecash"];
    final amount = widget.tx.amount;

    if (ecash != null && amount > BigInt.zero) {
      invoicePaidToastVisible.value = false;
      await showAppModalBottomSheet(
        context: context,
        child: EcashRedeemPrompt(
          fed: widget.fed,
          ecash: ecash,
          amount: amount,
        ),
        heightFactor: 0.33,
      );
      invoicePaidToastVisible.value = true;
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Icon(
              widget.icon.icon,
              color: theme.colorScheme.primary,
              size: 24,
            ),
            const SizedBox(width: 8),
            Text(
              _getTitleFromKind(),
              style: theme.textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.bold,
                color: theme.colorScheme.primary,
              ),
            ),
          ],
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
            children: widget.details.entries.map((entry) {
              final abbreviate = entry.key == "Ecash";
              return CopyableDetailRow(
                label: entry.key,
                value: entry.value,
                abbreviate: abbreviate,
              );
            }).toList(),
          ),
        ),
        if (widget.tx.kind is TransactionKind_EcashSend) ...[
          const SizedBox(height: 24),
          Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              ElevatedButton(
                onPressed: _checking ? null : _checkClaimStatus,
                style: ElevatedButton.styleFrom(
                  backgroundColor: theme.colorScheme.primary,
                  foregroundColor: Colors.black,
                  padding: const EdgeInsets.symmetric(vertical: 16),
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(12),
                  ),
                ),
                child: _checking
                    ? const SizedBox(
                        width: 20,
                        height: 20,
                        child: CircularProgressIndicator(
                          strokeWidth: 2,
                          valueColor: AlwaysStoppedAnimation<Color>(Colors.black),
                        ),
                      )
                    : const Text("Check Claim Status"),
              ),
              const SizedBox(height: 16),
              OutlinedButton(
                onPressed: _redeemEcash,
                style: OutlinedButton.styleFrom(
                  foregroundColor: theme.colorScheme.primary,
                  side: BorderSide(color: theme.colorScheme.primary),
                  padding: const EdgeInsets.symmetric(vertical: 16),
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(12),
                  ),
                ),
                child: const Text("Redeem Ecash"),
              ),
            ],
          ),
        ],
      ],
    );
  }
}

