import 'dart:convert';

import 'package:carbine/fed_preview.dart';
import 'package:carbine/lib.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/pay_preview.dart';
import 'package:carbine/redeem_ecash.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/utils.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

class ScanQRPage extends StatefulWidget {
  final FederationSelector? selectedFed;
  const ScanQRPage({super.key, this.selectedFed});

  @override
  State<ScanQRPage> createState() => _ScanQRPageState();
}

class _ScanQRPageState extends State<ScanQRPage> {
  bool _scanned = false;
  bool _isPasting = false;

  _QrLoopSession? _currentSession;

  Future<void> _processText(String text) async {
    if (text.startsWith("fed") &&
        !text.startsWith("fedimint") &&
        widget.selectedFed == null) {
      final meta = await getFederationMeta(inviteCode: text);

      final fed = await showCarbineModalBottomSheet(
        context: context,
        child: FederationPreview(
          federationName: meta.selector.federationName,
          inviteCode: meta.selector.inviteCode,
          welcomeMessage: meta.welcome,
          imageUrl: meta.picture,
          joinable: true,
          guardians: meta.guardians,
          network: meta.selector.network!,
        ),
      );

      if (fed != null) {
        await Future.delayed(const Duration(milliseconds: 400));
        Navigator.pop(context, fed);
      }
    } else if (text.startsWith("ln")) {
      if (widget.selectedFed != null) {
        final preview = await paymentPreview(
          federationId: widget.selectedFed!.federationId,
          bolt11: text,
        );
        if (widget.selectedFed!.network != preview.network) {
          AppLogger.instance.warn(
            "Widget network: ${widget.selectedFed!.network} Preview: ${preview.network}",
          );
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text("Cannot pay invoice from different network."),
            ),
          );
          return;
        }
        final bal = await balance(
          federationId: widget.selectedFed!.federationId,
        );
        if (bal < preview.amountMsats) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text(
                "This federation does not have enough funds to pay this invoice",
              ),
            ),
          );
          return;
        }

        showCarbineModalBottomSheet(
          context: context,
          child: PaymentPreviewWidget(
            fed: widget.selectedFed!,
            paymentPreview: preview,
          ),
        );
      }
    } else {
      if (widget.selectedFed != null) {
        try {
          final amountMsats = await parseEcash(
            federationId: widget.selectedFed!.federationId,
            ecash: text,
          );
          showCarbineModalBottomSheet(
            context: context,
            child: EcashRedeemPrompt(
              fed: widget.selectedFed!,
              ecash: text,
              amount: amountMsats,
            ),
            heightFactor: 0.33,
          );
        } catch (_) {
          AppLogger.instance.error('Could not parse text as ecash');
        }
      } else {
        AppLogger.instance.warn("Scanned unknown Text");
      }
    }
  }

  void _handleQrLoopChunk(String jsonChunk) {
    try {
      final Map<String, dynamic> parsed = json.decode(jsonChunk);
      final id = parsed['id'];
      final total = parsed['total'];
      final index = parsed['index'];
      final payload = parsed['payload'];

      if (id is! String ||
          total is! int ||
          index is! int ||
          payload is! String) {
        AppLogger.instance.warn("Scanned QR has invalid data: $jsonChunk");
        return;
      }

      AppLogger.instance.info("Scanned: index: $index / $total");

      // Reset if new session
      if (_currentSession == null || _currentSession!.id != id) {
        _currentSession = _QrLoopSession(id: id, total: total);
      }

      final session = _currentSession!;
      if (!session.receivedChunks.containsKey(index)) {
        session.receivedChunks[index] = payload;
        setState(() {}); // Triggers progress bar update
      }

      if (session.isComplete && !_scanned) {
        _scanned = true;
        final merged =
            List.generate(total, (i) => session.receivedChunks[i] ?? '').join();
        _processText(merged);
      }
    } catch (_) {
      AppLogger.instance.info("NOT A CHUNKED QR: $jsonChunk");
      if (!_scanned) _onQRCodeScanned(jsonChunk);
    }
  }

  void _onQRCodeScanned(String code) async {
    if (_scanned) return;
    _scanned = true;
    await _processText(code);
  }

  Future<void> _pasteFromClipboard() async {
    setState(() {
      _isPasting = true;
    });

    final clipboardData = await Clipboard.getData('text/plain');
    final text = clipboardData?.text ?? '';

    if (text.isEmpty) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(const SnackBar(content: Text("Clipboard is empty")));
      setState(() => _isPasting = false);
      return;
    }

    await _processText(text);
    setState(() => _isPasting = false);
  }

  double? get _progress {
    final session = _currentSession;
    if (session == null || session.total <= 1) return null;
    return session.receivedChunks.length / session.total;
  }

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Scaffold(
        appBar: AppBar(
          title: const Text(
            'Scan QR',
            style: TextStyle(fontWeight: FontWeight.bold),
          ),
          centerTitle: true,
          backgroundColor: Colors.transparent,
          elevation: 0,
          leading: IconButton(
            icon: const Icon(Icons.close),
            onPressed: () => Navigator.of(context).pop(),
          ),
        ),
        body: Stack(
          children: [
            Positioned.fill(
              child: MobileScanner(
                onDetect: (capture) {
                  final barcode = capture.barcodes.first;
                  final String? code = barcode.rawValue;
                  if (code != null) {
                    _handleQrLoopChunk(code);
                  }
                },
              ),
            ),
            if (_progress != null)
              Align(
                alignment: Alignment.topCenter,
                child: Padding(
                  padding: const EdgeInsets.all(32.0),
                  child: TweenAnimationBuilder<double>(
                    duration: const Duration(milliseconds: 300),
                    tween: Tween<double>(begin: 0, end: _progress!),
                    builder: (context, value, child) {
                      final received =
                          _currentSession?.receivedChunks.length ?? 0;
                      final total = _currentSession?.total ?? 0;

                      return Stack(
                        alignment: Alignment.center,
                        children: [
                          SizedBox(
                            width: 100,
                            height: 100,
                            child: CircularProgressIndicator(
                              value: value,
                              strokeWidth: 8,
                              backgroundColor: Colors.grey.shade800,
                              valueColor: const AlwaysStoppedAnimation<Color>(
                                Colors.greenAccent,
                              ),
                            ),
                          ),
                          Text(
                            "$received / $total",
                            style: const TextStyle(
                              fontSize: 16,
                              fontWeight: FontWeight.bold,
                              color: Colors.white,
                            ),
                          ),
                        ],
                      );
                    },
                  ),
                ),
              ),
            Align(
              alignment: Alignment.bottomCenter,
              child: Padding(
                padding: const EdgeInsets.all(24.0),
                child: ElevatedButton.icon(
                  onPressed: _isPasting ? null : _pasteFromClipboard,
                  icon:
                      _isPasting
                          ? const SizedBox(
                            width: 20,
                            height: 20,
                            child: CircularProgressIndicator(
                              color: Colors.white,
                              strokeWidth: 2.0,
                            ),
                          )
                          : const Icon(Icons.paste),
                  label: Text(
                    _isPasting ? "Pasting..." : "Paste from Clipboard",
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

class _QrLoopSession {
  final String id;
  final int total;
  final Map<int, String> receivedChunks = {};
  _QrLoopSession({required this.id, required this.total});

  bool get isComplete => receivedChunks.length >= total;
}
