import 'package:carbine/lib.dart';
import 'package:carbine/success.dart';
import 'package:flutter/material.dart';

class SendPayment extends StatefulWidget {
  final FederationSelector fed;
  final String? invoice;
  final String? lnAddress;
  final BigInt amountMsats;

  const SendPayment({
    super.key,
    required this.fed,
    required this.amountMsats,
    this.invoice,
    this.lnAddress,
  });

  @override
  State<SendPayment> createState() => _SendPaymentState();
}

class _SendPaymentState extends State<SendPayment> {
  bool _isSending = true;

  @override
  void initState() {
    super.initState();
    _payInvoice();
  }

  Future<OperationId> _sendPayment() async {
    if (widget.invoice != null) {
      final operationId = await send(
        federationId: widget.fed.federationId,
        invoice: widget.invoice!,
      );
      return operationId;
    } else {
      final operationId = await sendLnaddress(
        federationId: widget.fed.federationId,
        amountMsats: widget.amountMsats,
        address: widget.lnAddress!,
      );
      return operationId;
    }
  }

  void _payInvoice() async {
    try {
      final operationId = await _sendPayment();
      final finalState = await awaitSend(
        federationId: widget.fed.federationId,
        operationId: operationId,
      );

      debugPrint('FinalState: $finalState');

      if (!mounted) return;

      setState(() {
        _isSending = false;
      });

      // Navigate to Success screen
      Navigator.push(
        context,
        MaterialPageRoute(
          builder:
              (context) => Success(
                lightning: true,
                received: false,
                amountMsats: widget.amountMsats,
              ),
        ),
      );

      await Future.delayed(const Duration(seconds: 4));

      if (mounted) {
        Navigator.of(context).popUntil((route) => route.isFirst);
      }
    } catch (e) {
      debugPrint('Error while sending payment: $e');
      if (!mounted) return;
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(const SnackBar(content: Text('Failed to send payment')));
      Navigator.of(context).pop(); // Close modal on failure
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Center(
      child: AnimatedSwitcher(
        duration: const Duration(milliseconds: 500),
        child:
            _isSending
                ? Column(
                  key: const ValueKey('sending'),
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const SizedBox(height: 24),
                    CircularProgressIndicator(
                      valueColor: AlwaysStoppedAnimation(
                        theme.colorScheme.primary,
                      ),
                      strokeWidth: 3,
                    ),
                    const SizedBox(height: 24),
                    Text(
                      'Sending Payment',
                      style: theme.textTheme.titleLarge?.copyWith(
                        fontWeight: FontWeight.bold,
                        color: theme.colorScheme.onSurface,
                      ),
                    ),
                  ],
                )
                : const SizedBox.shrink(), // Replaced by Success screen
      ),
    );
  }
}
