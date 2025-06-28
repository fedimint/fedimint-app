import 'package:carbine/models.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/scan.dart';
import 'package:carbine/send.dart';
import 'package:carbine/utils.dart';
import 'package:flutter/material.dart';

class PaymentMethodSelector extends StatefulWidget {
  final FederationSelector fed;
  const PaymentMethodSelector({super.key, required this.fed});

  @override
  State<PaymentMethodSelector> createState() => _PaymentMethodSelectorState();
}

class _PaymentMethodSelectorState extends State<PaymentMethodSelector> {
  String _selected = 'invoice';
  final _lightningAddressController = TextEditingController();
  final _amountController = TextEditingController();

  bool _isLightningFormValid = false;

  @override
  void initState() {
    super.initState();
    _lightningAddressController.addListener(_validateLightningForm);
    _amountController.addListener(_validateLightningForm);
  }

  void _validateLightningForm() {
    final address = _lightningAddressController.text.trim();
    final amount = int.tryParse(_amountController.text.trim()) ?? 0;

    final isValid = address.contains('@') && amount > 0;
    if (isValid != _isLightningFormValid) {
      setState(() {
        _isLightningFormValid = isValid;
      });
    }
  }

  @override
  void dispose() {
    _lightningAddressController.dispose();
    _amountController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          'Choose Payment Method',
          textAlign: TextAlign.center,
          style: theme.textTheme.titleLarge?.copyWith(
            fontWeight: FontWeight.bold,
          ),
        ),
        const SizedBox(height: 24),

        // Segmented control
        Container(
          decoration: BoxDecoration(
            color: isDark ? Colors.grey[900] : Colors.grey[200],
            borderRadius: BorderRadius.circular(16),
          ),
          padding: const EdgeInsets.all(4),
          child: Row(
            children: [
              _buildOption(
                label: 'Invoice',
                icon: Icons.qr_code,
                value: 'invoice',
              ),
              const SizedBox(width: 4),
              _buildOption(
                label: 'Lightning Address',
                icon: Icons.flash_on,
                value: 'lnaddress',
              ),
            ],
          ),
        ),

        const SizedBox(height: 32),

        // Content without animation
        if (_selected == 'invoice')
          _buildInvoiceInstructions()
        else
          _buildLightningAddressInstructions(),

        const SizedBox(height: 32),

        // Confirm Button
        ElevatedButton.icon(
          onPressed:
              (_selected == 'invoice' || _isLightningFormValid)
                  ? _onConfirmPressed
                  : null,
          icon: const Icon(Icons.check_circle),
          label: const Text('Confirm'),
          style: ElevatedButton.styleFrom(
            backgroundColor: theme.colorScheme.primary,
            foregroundColor: theme.colorScheme.onPrimary,
            padding: const EdgeInsets.symmetric(vertical: 16),
            textStyle: theme.textTheme.titleMedium?.copyWith(
              fontWeight: FontWeight.w600,
            ),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(16),
            ),
          ),
        ),
      ],
    );
  }

  void _onConfirmPressed() async {
    if (_selected == 'lnaddress') {
      try {
        final address = _lightningAddressController.text;
        final amount = BigInt.parse(_amountController.text) * BigInt.from(1000);
        AppLogger.instance.info('Lightning Address: $address, Amount: $amount');
        await Navigator.push(
          context,
          MaterialPageRoute(
            builder:
                (context) => SendPayment(
                  fed: widget.fed,
                  amountMsats: amount,
                  lnAddress: address,
                ),
          ),
        );
      } catch (e) {
        AppLogger.instance.error("Error paying lightning address $e");
      }
    } else {
      await Navigator.push(
        context,
        MaterialPageRoute(
          builder:
              (context) => ScanQRPage(
                selectedFed: widget.fed,
                paymentType: PaymentType.lightning,
              ),
        ),
      );
    }
  }

  Widget _buildOption({
    required String label,
    required IconData icon,
    required String value,
  }) {
    final theme = Theme.of(context);
    final isSelected = _selected == value;

    return Expanded(
      child: GestureDetector(
        onTap:
            () => setState(() {
              _selected = value;
            }),
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 200),
          padding: const EdgeInsets.symmetric(vertical: 16),
          decoration: BoxDecoration(
            color: isSelected ? theme.colorScheme.primary : Colors.transparent,
            borderRadius: BorderRadius.circular(12),
          ),
          child: Column(
            children: [
              Icon(
                icon,
                color: isSelected ? Colors.white : theme.colorScheme.onSurface,
              ),
              const SizedBox(height: 8),
              Text(
                label,
                style: theme.textTheme.bodyMedium?.copyWith(
                  color:
                      isSelected ? Colors.white : theme.colorScheme.onSurface,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildInvoiceInstructions() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        Icon(Icons.qr_code, size: 64, color: Colors.greenAccent),
        const SizedBox(height: 16),
        Text(
          'Scan or paste a Bolt11 invoice.',
          textAlign: TextAlign.center,
          style: Theme.of(context).textTheme.bodyLarge,
        ),
      ],
    );
  }

  Widget _buildLightningAddressInstructions() {
    final theme = Theme.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Icon(Icons.flash_on, size: 64, color: Colors.amber),
        const SizedBox(height: 16),
        Text(
          'Enter a Lightning Address and Amount.',
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyLarge,
        ),
        const SizedBox(height: 24),
        TextField(
          controller: _lightningAddressController,
          decoration: InputDecoration(
            labelText: 'Lightning Address',
            border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
            prefixIcon: const Icon(Icons.alternate_email),
          ),
          keyboardType: TextInputType.emailAddress,
        ),
        const SizedBox(height: 16),
        TextField(
          controller: _amountController,
          decoration: InputDecoration(
            labelText: 'Amount (sats)',
            border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
            prefixIcon: const Icon(Icons.currency_bitcoin),
          ),
          keyboardType: TextInputType.number,
        ),
      ],
    );
  }
}
