import 'dart:async';
import 'package:carbine/lib.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:qr_flutter/qr_flutter.dart';

class EcashSend extends StatefulWidget {
  final FederationSelector fed;
  final BigInt amountSats;

  const EcashSend({super.key, required this.fed, required this.amountSats});

  @override
  State<EcashSend> createState() => _EcashSendState();
}

class _EcashSendState extends State<EcashSend> {
  String? _ecash;
  bool _loading = true;
  BigInt _ecashAmountMsats = BigInt.zero;
  bool _reclaiming = false;

  double _progress = 0.0;
  Timer? _holdTimer;

  static const Duration _holdDuration = Duration(seconds: 1);

  @override
  void initState() {
    super.initState();
    _loadEcash();
  }

  Future<void> _loadEcash() async {
    try {
      final ecash = await sendEcash(
        federationId: widget.fed.federationId,
        amountMsats: widget.amountSats * BigInt.from(1000),
      );

      setState(() {
        _ecash = ecash.$2;
        _ecashAmountMsats = ecash.$3 ~/ BigInt.from(1000);
        _loading = false;
      });
    } catch (_) {
      setState(() {
        _ecash = null;
        _loading = false;
      });
    }
  }

  Future<void> _reclaimEcash() async {
    setState(() => _reclaiming = true);

    if (_ecash != null) {
      final opId = await reissueEcash(
        federationId: widget.fed.federationId,
        ecash: _ecash!,
      );
      await awaitEcashReissue(
        federationId: widget.fed.federationId,
        operationId: opId,
      );
      if (mounted) {
        Navigator.of(context).popUntil((route) => route.isFirst);
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(const SnackBar(content: Text('✅ Ecash reclaimed')));
      }
    }

    setState(() => _reclaiming = false);
  }

  void _startHold() {
    _progress = 0;
    const tick = Duration(milliseconds: 20);
    int elapsed = 0;
    _holdTimer = Timer.periodic(tick, (timer) {
      elapsed += tick.inMilliseconds;
      final progress = elapsed / _holdDuration.inMilliseconds;
      if (progress >= 1.0) {
        timer.cancel();
        _copyEcash();
        setState(() => _progress = 1.0);
      } else {
        setState(() => _progress = progress);
      }
    });
  }

  void _cancelHold() {
    _holdTimer?.cancel();
    setState(() => _progress = 0.0);
  }

  void _copyEcash() {
    Clipboard.setData(ClipboardData(text: _ecash!));
    Navigator.of(context).popUntil((route) => route.isFirst);
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('✅ Ecash copied to clipboard')),
    );
  }

  // TODO: Remove this, it is confusing when sending
  String _abbreviate(String text, [int max = 30]) {
    if (text.length <= max) return text;
    return '${text.substring(0, 10)}...${text.substring(text.length - 10)}';
  }

  @override
  void dispose() {
    _holdTimer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (_loading) {
      return const Center(child: CircularProgressIndicator());
    }

    if (_ecash == null) {
      return const Center(child: Text("⚠️ Failed to load ecash"));
    }

    return Padding(
      padding: const EdgeInsets.all(24),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(Icons.lock_outline, size: 48),
            const SizedBox(height: 12),
            Text(
              "Ecash Withdrawn",
              style: Theme.of(context).textTheme.headlineSmall,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 8),
            Text(
              "You’ve withdrawn ${_ecashAmountMsats.toString()} sats.\n"
              "You must now send this ecash string to the recipient.",
              textAlign: TextAlign.center,
              style: Theme.of(context).textTheme.bodyMedium,
            ),
            const SizedBox(height: 24),

            // QR Code
            Card(
              margin: EdgeInsets.zero,
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(16),
              ),
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: QrImageView(
                  data: _ecash!,
                  version: QrVersions.auto,
                  size: 240,
                  backgroundColor: Colors.white,
                ),
              ),
            ),
            const SizedBox(height: 24),

            // Abbreviated text field
            TextField(
              readOnly: true,
              controller: TextEditingController(text: _abbreviate(_ecash!)),
              decoration: const InputDecoration(
                labelText: 'Ecash (abbreviated)',
                prefixIcon: Icon(Icons.key),
              ),
            ),
            const SizedBox(height: 24),

            // Hold-to-copy button
            GestureDetector(
              onTapDown: (_) => _startHold(),
              onTapUp: (_) => _cancelHold(),
              onTapCancel: () => _cancelHold(),
              child: Stack(
                alignment: Alignment.center,
                children: [
                  SizedBox(
                    width: 80,
                    height: 80,
                    child: CircularProgressIndicator(
                      value: _progress,
                      strokeWidth: 6,
                    ),
                  ),
                  Container(
                    width: 60,
                    height: 60,
                    decoration: const BoxDecoration(
                      shape: BoxShape.circle,
                      color: Colors.green,
                    ),
                    alignment: Alignment.center,
                    child: const Icon(Icons.copy, color: Colors.black),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 24),

            // Reclaim button
            _reclaiming
                ? const CircularProgressIndicator()
                : SizedBox(
                  width: double.infinity,
                  child: ElevatedButton.icon(
                    onPressed: _reclaimEcash,
                    icon: const Icon(Icons.undo),
                    label: const Text("Reclaim"),
                    style: ElevatedButton.styleFrom(
                      backgroundColor: Colors.red,
                      foregroundColor: Colors.white,
                      padding: const EdgeInsets.symmetric(vertical: 16),
                    ),
                  ),
                ),
          ],
        ),
      ),
    );
  }
}
