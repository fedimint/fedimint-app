import 'package:carbine/lib.dart';
import 'package:carbine/main.dart';
import 'package:carbine/success.dart';
import 'package:flutter/material.dart';

class EcashRedeemPrompt extends StatefulWidget {
  final FederationSelector fed;
  final String ecash;
  final BigInt amount;

  const EcashRedeemPrompt({
    super.key,
    required this.fed,
    required this.ecash,
    required this.amount,
  });

  @override
  State<EcashRedeemPrompt> createState() => _EcashRedeemPromptState();
}

class _EcashRedeemPromptState extends State<EcashRedeemPrompt> {
  bool _isLoading = false;

  Future<void> _handleRedeem() async {
    print("Handling redeem...");
    setState(() {
      _isLoading = true;
    });

    try {
      final operationId = await reissueEcash(
        federationId: widget.fed.federationId,
        ecash: widget.ecash,
      );
      await awaitEcashReissue(
        federationId: widget.fed.federationId,
        operationId: operationId,
      );

      if (!mounted) return;

      Navigator.push(
        context,
        MaterialPageRoute(
          builder:
              (context) => Success(
                lightning: false,
                received: true,
                amountMsats: widget.amount,
              ),
        ),
      );
      await Future.delayed(const Duration(seconds: 4));
      Navigator.of(context).popUntil((route) => route.isFirst);
    } catch (_) {
      // Could not reissue ecash
      print("Could not reissue ecash");
      Navigator.of(context).popUntil((route) => route.isFirst);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          'Do you want to redeem the following ecash?',
          style: theme.textTheme.titleLarge,
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 16),
        Text(
          formatBalance(widget.amount, false),
          textAlign: TextAlign.center,
          style: theme.textTheme.displaySmall?.copyWith(
            fontWeight: FontWeight.bold,
            fontSize: 32,
            color: Colors.greenAccent,
            letterSpacing: 1.5,
            shadows: [
              Shadow(
                blurRadius: 8,
                color: Colors.greenAccent.withOpacity(0.4),
                offset: const Offset(0, 0),
              ),
            ],
          ),
        ),
        const SizedBox(height: 32),
        SizedBox(
          width: double.infinity,
          child: ElevatedButton(
            onPressed: _isLoading ? null : _handleRedeem,
            style: ElevatedButton.styleFrom(
              backgroundColor: theme.colorScheme.primary,
              foregroundColor: Colors.black,
              padding: const EdgeInsets.symmetric(vertical: 16),
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(12),
              ),
            ),
            child:
                _isLoading
                    ? const SizedBox(
                      height: 20,
                      width: 20,
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        valueColor: AlwaysStoppedAnimation<Color>(Colors.black),
                      ),
                    )
                    : const Text('Confirm'),
          ),
        ),
      ],
    );
  }
}
