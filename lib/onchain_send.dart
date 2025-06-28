import 'dart:async';
import 'package:carbine/lib.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/number_pad.dart';
import 'package:carbine/success.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/utils.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

class OnchainSend extends StatefulWidget {
  final FederationSelector fed;
  final BigInt amountSats;
  final WithdrawalMode withdrawalMode;
  final VoidCallback? onWithdrawCompleted;
  final String? defaultAddress;

  const OnchainSend({
    super.key,
    required this.fed,
    required this.amountSats,
    required this.withdrawalMode,
    this.onWithdrawCompleted,
    this.defaultAddress,
  });

  @override
  State<OnchainSend> createState() => _OnchainSendState();
}

class _OnchainSendState extends State<OnchainSend> {
  late TextEditingController _addressController;
  String? _feeQuote;
  BigInt? _feeAmountSats;
  double? _feeRateSatsPerVbyte;
  int? _txSizeVbytes;
  BigInt? _actualWithdrawalAmount;
  PegOutFees? _pegOutFees;
  bool _loadingFees = false;
  bool _withdrawing = false;
  DateTime? _quoteExpiry;
  Timer? _quoteTimer;

  @override
  void initState() {
    super.initState();
    _addressController = TextEditingController(
      text: widget.defaultAddress ?? '',
    );
    _addressController.addListener(_onAddressChanged);
  }

  @override
  void dispose() {
    _addressController.dispose();
    _quoteTimer?.cancel();
    super.dispose();
  }

  void _onAddressChanged() {
    if (_feeQuote != null) {
      setState(() {
        _feeQuote = null;
        _feeAmountSats = null;
        _feeRateSatsPerVbyte = null;
        _txSizeVbytes = null;
        _actualWithdrawalAmount = null;
        _pegOutFees = null;
        _quoteExpiry = null;
      });
      _quoteTimer?.cancel();
    }
  }

  Future<void> _calculateFees() async {
    if (_addressController.text.isEmpty) return;
    _quoteTimer?.cancel();
    setState(() => _loadingFees = true);

    try {
      if (widget.withdrawalMode == WithdrawalMode.maxBalance) {
        final maxAmount = await getMaxWithdrawableAmount(
          federationId: widget.fed.federationId,
          address: _addressController.text.trim(),
        );

        final feesResponse = await calculateWithdrawFees(
          federationId: widget.fed.federationId,
          address: _addressController.text.trim(),
          amountSats: maxAmount,
        );

        setState(() {
          _actualWithdrawalAmount = maxAmount;
          _feeAmountSats = feesResponse.feeAmount;
          _feeRateSatsPerVbyte = feesResponse.feeRateSatsPerVb;
          _txSizeVbytes = feesResponse.txSizeVbytes;
          _pegOutFees = feesResponse.pegOutFees;
          _feeQuote = 'Fee calculated';
          _quoteExpiry = DateTime.now().add(const Duration(seconds: 60));
        });
      } else {
        final feesResponse = await calculateWithdrawFees(
          federationId: widget.fed.federationId,
          address: _addressController.text.trim(),
          amountSats: widget.amountSats,
        );

        setState(() {
          _actualWithdrawalAmount = widget.amountSats;
          _feeAmountSats = feesResponse.feeAmount;
          _feeRateSatsPerVbyte = feesResponse.feeRateSatsPerVb;
          _txSizeVbytes = feesResponse.txSizeVbytes;
          _pegOutFees = feesResponse.pegOutFees;
          _feeQuote = 'Fee calculated';
          _quoteExpiry = DateTime.now().add(const Duration(seconds: 60));
        });
      }

      _startQuoteTimer();
    } catch (e) {
      AppLogger.instance.error('Failed to calculate fees: $e');
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text('Failed to calculate fees: $e')));
    } finally {
      setState(() => _loadingFees = false);
    }
  }

  void _startQuoteTimer() {
    _quoteTimer?.cancel();
    _quoteTimer = Timer.periodic(const Duration(seconds: 1), (timer) {
      if (_quoteExpiry != null && DateTime.now().isAfter(_quoteExpiry!)) {
        setState(() {
          _feeQuote = null;
          _feeAmountSats = null;
          _feeRateSatsPerVbyte = null;
          _txSizeVbytes = null;
          _actualWithdrawalAmount = null;
          _pegOutFees = null;
          _quoteExpiry = null;
        });
        timer.cancel();
      } else {
        // refresh triggers an update for the countdown
        setState(() {});
      }
    });
  }

  String _getQuoteTimeRemaining() {
    if (_quoteExpiry == null) return '';
    final remaining = _quoteExpiry!.difference(DateTime.now()).inSeconds;
    if (remaining <= 0) return 'Expired';
    return '${remaining}s remaining';
  }

  String _formatFeeRate(double? feeRate) {
    if (feeRate == null) return '';
    // format to up to 3 decimal places
    return '${feeRate.toStringAsFixed(3).replaceAll(RegExp(r'\.?0+$'), '')} sats/vB';
  }

  Future<void> _withdraw() async {
    if (_addressController.text.isEmpty ||
        _pegOutFees == null ||
        _actualWithdrawalAmount == null) {
      return;
    }

    setState(() => _withdrawing = true);

    try {
      final operationId = await withdrawToAddress(
        federationId: widget.fed.federationId,
        address: _addressController.text.trim(),
        amountSats: _actualWithdrawalAmount!,
        pegOutFees: _pegOutFees!,
      );

      final txid = await awaitWithdraw(
        federationId: widget.fed.federationId,
        operationId: operationId,
      );

      if (mounted) {
        Navigator.of(context).push(
          MaterialPageRoute(
            builder:
                (context) => Success(
                  lightning: false,
                  received: false,
                  amountMsats:
                      (_actualWithdrawalAmount ?? widget.amountSats) *
                      BigInt.from(1000),
                  txid: txid,
                  onCompleted: widget.onWithdrawCompleted,
                ),
          ),
        );
      }
    } catch (e) {
      AppLogger.instance.error('Failed to withdraw: $e');
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text('Withdrawal failed: $e')));
      }
    } finally {
      if (mounted) {
        setState(() => _withdrawing = false);
      }
    }
  }

  void _pasteFromClipboard() async {
    final clipboardData = await Clipboard.getData(Clipboard.kTextPlain);
    if (clipboardData?.text != null) {
      _addressController.text = clipboardData!.text!;
    }
  }

  @override
  Widget build(BuildContext context) {
    final canWithdraw =
        _feeQuote != null &&
        _actualWithdrawalAmount != null &&
        _pegOutFees != null &&
        _quoteExpiry != null &&
        DateTime.now().isBefore(_quoteExpiry!) &&
        !_withdrawing;

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // Header section
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 16),
          child: Column(
            children: [
              Icon(
                Icons.outbound,
                size: 48,
                color: Theme.of(context).colorScheme.primary,
              ),
              const SizedBox(height: 12),
              Text(
                formatBalance(widget.amountSats * BigInt.from(1000), false),
                style: Theme.of(context).textTheme.headlineSmall,
                textAlign: TextAlign.center,
              ),
            ],
          ),
        ),

        // Form section
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 24),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              // Address input
              TextField(
                controller: _addressController,
                style: const TextStyle(fontFamily: 'monospace', fontSize: 14),
                decoration: InputDecoration(
                  labelText: 'Bitcoin Address',
                  hintText: 'Enter destination address',
                  suffixIcon: IconButton(
                    icon: const Icon(Icons.paste),
                    onPressed: _pasteFromClipboard,
                    tooltip: 'Paste',
                  ),
                ),
                maxLines: 1,
              ),
              const SizedBox(height: 16),

              // Calculate fees button
              if (_feeQuote == null)
                ElevatedButton(
                  onPressed: _loadingFees ? null : _calculateFees,
                  child:
                      _loadingFees
                          ? const Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              SizedBox(
                                width: 16,
                                height: 16,
                                child: CircularProgressIndicator(
                                  strokeWidth: 2,
                                ),
                              ),
                              SizedBox(width: 8),
                              Text('Calculating...'),
                            ],
                          )
                          : const Text('Calculate Fees'),
                ),

              // Fee quote display
              if (_feeQuote != null) ...[
                const SizedBox(height: 16),
                Text(
                  'Withdrawal Quote',
                  style: Theme.of(context).textTheme.titleMedium?.copyWith(
                    color: Theme.of(context).colorScheme.onSurface,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const SizedBox(height: 12),
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(16),
                  decoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.surfaceContainer,
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(
                      color: Theme.of(
                        context,
                      ).colorScheme.primary.withOpacity(0.25),
                    ),
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      buildDetailRow(
                        Theme.of(context),
                        'Amount',
                        formatBalance(
                          _actualWithdrawalAmount! * BigInt.from(1000),
                          false,
                        ),
                      ),
                      buildDetailRow(
                        Theme.of(context),
                        'Fee Rate',
                        _formatFeeRate(_feeRateSatsPerVbyte),
                      ),
                      buildDetailRow(
                        Theme.of(context),
                        'Tx Size',
                        '${_txSizeVbytes ?? 0} vB',
                      ),
                      buildDetailRow(
                        Theme.of(context),
                        'Fee',
                        formatBalance(
                          _feeAmountSats! * BigInt.from(1000),
                          false,
                        ),
                      ),
                      buildDetailRow(
                        Theme.of(context),
                        'Total',
                        formatBalance(
                          (_actualWithdrawalAmount! + _feeAmountSats!) *
                              BigInt.from(1000),
                          false,
                        ),
                      ),
                      Padding(
                        padding: const EdgeInsets.only(top: 8),
                        child: Text(
                          _getQuoteTimeRemaining(),
                          style: TextStyle(
                            color:
                                _quoteExpiry != null &&
                                        DateTime.now().isAfter(_quoteExpiry!)
                                    ? Colors.red
                                    : Theme.of(context).colorScheme.primary,
                            fontWeight: FontWeight.bold,
                            fontSize: 14,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 16),
              ],

              // Action buttons
              if (_feeQuote != null) ...[
                const SizedBox(height: 24),
                Row(
                  children: [
                    Expanded(
                      child: TextButton(
                        onPressed: _loadingFees ? null : _calculateFees,
                        child:
                            _loadingFees
                                ? const Row(
                                  mainAxisSize: MainAxisSize.min,
                                  children: [
                                    SizedBox(
                                      width: 14,
                                      height: 14,
                                      child: CircularProgressIndicator(
                                        strokeWidth: 2,
                                      ),
                                    ),
                                    SizedBox(width: 6),
                                    Text('Updating...'),
                                  ],
                                )
                                : const Text('Recalculate'),
                      ),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: ElevatedButton(
                        onPressed: canWithdraw ? _withdraw : null,
                        style: ElevatedButton.styleFrom(
                          backgroundColor:
                              Theme.of(context).colorScheme.primary,
                          foregroundColor: Colors.black,
                          padding: const EdgeInsets.symmetric(vertical: 16),
                        ),
                        child:
                            _withdrawing
                                ? const SizedBox(
                                  height: 20,
                                  width: 20,
                                  child: CircularProgressIndicator(
                                    color: Colors.black,
                                    strokeWidth: 2,
                                  ),
                                )
                                : const Text('Confirm Withdrawal'),
                      ),
                    ),
                  ],
                ),
              ],
              const SizedBox(height: 16),
            ],
          ),
        ),
      ],
    );
  }
}
