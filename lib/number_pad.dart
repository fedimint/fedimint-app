import 'dart:convert';

import 'package:carbine/ecash_send.dart';
import 'package:carbine/lib.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/request.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/utils.dart';
import 'package:carbine/models.dart';
import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;
import 'package:intl/intl.dart';
import 'package:numpad_layout/widgets/numpad.dart';
import 'package:flutter/services.dart';

class NumberPad extends StatefulWidget {
  final FederationSelector fed;
  final PaymentType paymentType;
  const NumberPad({super.key, required this.fed, required this.paymentType});

  @override
  State<NumberPad> createState() => _NumberPadState();
}

class _NumberPadState extends State<NumberPad> {
  final FocusNode _numpadFocus = FocusNode();

  String _rawAmount = '';
  bool _creating = false;
  double? _btcPriceUsd;

  @override
  void initState() {
    super.initState();
    _fetchPrice();

    WidgetsBinding.instance.addPostFrameCallback((_) {
      _numpadFocus.requestFocus();
    });
  }

  @override
  void dispose() {
    _numpadFocus.dispose();
    super.dispose();
  }

  Future<void> _fetchPrice() async {
    try {
      final uri = Uri.parse('https://mempool.space/api/v1/prices');
      final response = await http.get(uri);

      if (response.statusCode == 200) {
        final data = json.decode(response.body);
        setState(() {
          _btcPriceUsd = (data['USD'] as num).toDouble();
        });
      } else {
        debugPrint('Failed to load price data');
      }
    } catch (e) {
      debugPrint('Error fetching price: $e');
    }
  }

  String _formatAmount(String value) {
    if (value.isEmpty) return '₿0';
    final number = int.tryParse(value) ?? 0;
    final formatter = NumberFormat('₿#,###', 'en_US');
    return formatter.format(number).replaceAll(',', ' ');
  }

  String _calculateUsdValue() {
    if (_btcPriceUsd == null) return '';
    final sats = int.tryParse(_rawAmount) ?? 0;
    final usdValue = (_btcPriceUsd! * sats) / 100000000;
    return '\$${usdValue.toStringAsFixed(2)}';
  }

  Future<void> _onConfirm() async {
    setState(() => _creating = true);
    final amountSats = BigInt.tryParse(_rawAmount);
    if (amountSats != null) {
      if (widget.paymentType == PaymentType.lightning) {
        final requestedAmountMsats = amountSats * BigInt.from(1000);
        final gateway = await selectReceiveGateway(
          federationId: widget.fed.federationId,
          amountMsats: requestedAmountMsats,
        );
        final contractAmount = gateway.$2;
        final invoice = await receive(
          federationId: widget.fed.federationId,
          amountMsatsWithFees: contractAmount,
          amountMsatsWithoutFees: requestedAmountMsats,
          gateway: gateway.$1,
          isLnv2: gateway.$3,
        );
        showCarbineModalBottomSheet(
          context: context,
          child: Request(
            invoice: invoice.$1,
            fed: widget.fed,
            operationId: invoice.$2,
            requestedAmountMsats: requestedAmountMsats,
            totalMsats: contractAmount,
            gateway: gateway.$1,
            pubkey: invoice.$3,
            paymentHash: invoice.$4,
            expiry: invoice.$5,
          ),
        );
      } else if (widget.paymentType == PaymentType.ecash) {
        showCarbineModalBottomSheet(
          context: context,
          child: EcashSend(fed: widget.fed, amountSats: amountSats),
        );
      } else if (widget.paymentType == PaymentType.onchain) {
        AppLogger.instance.info('Generate bitcoin address and QR code');
      }
    }
    setState(() => _creating = false);
  }

  void _handleKeyEvent(KeyEvent event) {
    // only handle on key down
    if (event is KeyDownEvent) {
      final key = event.logicalKey;
      // Handle Enter for confirm
      if (key == LogicalKeyboardKey.enter ||
          key == LogicalKeyboardKey.numpadEnter) {
        _onConfirm();
        return;
      }

      String digit = '';
      if (key == LogicalKeyboardKey.digit0 ||
          key == LogicalKeyboardKey.numpad0) {
        digit = '0';
      }
      if (key == LogicalKeyboardKey.digit1 ||
          key == LogicalKeyboardKey.numpad1) {
        digit = '1';
      }
      if (key == LogicalKeyboardKey.digit2 ||
          key == LogicalKeyboardKey.numpad2) {
        digit = '2';
      }
      if (key == LogicalKeyboardKey.digit3 ||
          key == LogicalKeyboardKey.numpad3) {
        digit = '3';
      }
      if (key == LogicalKeyboardKey.digit4 ||
          key == LogicalKeyboardKey.numpad4) {
        digit = '4';
      }
      if (key == LogicalKeyboardKey.digit5 ||
          key == LogicalKeyboardKey.numpad5) {
        digit = '5';
      }
      if (key == LogicalKeyboardKey.digit6 ||
          key == LogicalKeyboardKey.numpad6) {
        digit = '6';
      }
      if (key == LogicalKeyboardKey.digit7 ||
          key == LogicalKeyboardKey.numpad7) {
        digit = '7';
      }
      if (key == LogicalKeyboardKey.digit8 ||
          key == LogicalKeyboardKey.numpad8) {
        digit = '8';
      }
      if (key == LogicalKeyboardKey.digit9 ||
          key == LogicalKeyboardKey.numpad9) {
        digit = '9';
      }
      if (key == LogicalKeyboardKey.backspace) {
        setState(() {
          if (_rawAmount.isNotEmpty) {
            _rawAmount = _rawAmount.substring(0, _rawAmount.length - 1);
          }
        });
      }
      if (digit != '') {
        setState(() => _rawAmount += digit);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final usdText = _calculateUsdValue();

    return SafeArea(
      child: Scaffold(
        appBar: AppBar(
          title: const Text(
            'Enter Amount',
            style: TextStyle(fontWeight: FontWeight.bold),
          ),
          centerTitle: true,
          backgroundColor: Colors.transparent,
          elevation: 0,
        ),
        body: Column(
          children: [
            const SizedBox(height: 24),
            RichText(
              text: TextSpan(
                style: const TextStyle(color: Colors.white),
                children: [
                  TextSpan(
                    text: _formatAmount(_rawAmount),
                    style: const TextStyle(
                      fontSize: 48,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 8),
            Text(
              usdText,
              style: const TextStyle(fontSize: 24, color: Colors.grey),
            ),
            const SizedBox(height: 24),
            Expanded(
              child: KeyboardListener(
                focusNode: _numpadFocus,
                onKeyEvent: _handleKeyEvent,
                child: Center(
                  child: NumPad(
                    arabicDigits: false,
                    onType: (value) {
                      setState(() {
                        _rawAmount += value.toString();
                      });
                    },
                    numberStyle: const TextStyle(
                      fontSize: 24,
                      color: Colors.grey,
                    ),
                    rightWidget: IconButton(
                      onPressed: () {
                        setState(() {
                          if (_rawAmount.isNotEmpty) {
                            _rawAmount = _rawAmount.substring(
                              0,
                              _rawAmount.length - 1,
                            );
                          }
                        });
                      },
                      icon: const Icon(Icons.backspace),
                    ),
                  ),
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.all(16.0),
              child: SizedBox(
                width: double.infinity,
                child: ElevatedButton(
                  onPressed: _onConfirm,
                  child:
                      _creating
                          ? const CircularProgressIndicator(color: Colors.white)
                          : const Text(
                            'Confirm',
                            style: TextStyle(fontSize: 20),
                          ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
