import 'package:carbine/multimint.dart';
import 'package:carbine/utils.dart';
import 'package:flutter/material.dart';
import 'package:carbine/lib.dart';
import 'package:flutter/services.dart';

class OnchainAddressesList extends StatefulWidget {
  final FederationSelector fed;
  final VoidCallback updateBalance;

  const OnchainAddressesList({super.key, required this.fed, required this.updateBalance});

  @override
  State<OnchainAddressesList> createState() => _OnchainAddressesListState();
}

class _OnchainAddressesListState extends State<OnchainAddressesList> {
  late Future<List<(String, BigInt, BigInt?)>> _addressesFuture;

  @override
  void initState() {
    super.initState();
    _addressesFuture = getAddresses(federationId: widget.fed.federationId);
  }

  String formatSats(BigInt amount) {
    return '${amount.toString()} sats';
  }

  String abbreviateAddress(String address, {int headLength = 8, int tailLength = 8}) {
    if (address.length <= headLength + tailLength) return address;
    final head = address.substring(0, headLength);
    final tail = address.substring(address.length - tailLength);
    return '$head...$tail';
  }

  String? _explorerUrl(String address) {
    switch (widget.fed.network) {
      case 'bitcoin':
        return 'https://mempool.space/address/$address';
      case 'signet':
        return 'https://mutinynet.com/address/$address';
      default:
        return null;
    }
  } 

  Future<void> _refreshAddress(BigInt tweakIdx, String address) async {
    try {
      // Call the Rust async function recheckAddress for the given address and federation
      await recheckAddress(federationId: widget.fed.federationId, tweakIdx: tweakIdx);
      widget.updateBalance();

      // Reload the addresses list
      //_addressesFuture = getAddresses(federationId: widget.fed.federationId);
      //setState(() {});

      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Rechecked ${abbreviateAddress(address)}')),
      );
    } catch (e) {
      AppLogger.instance.error("Failed to refresh address: $e");
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Failed to refresh address')),
      );
    }
  }



  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<(String, BigInt, BigInt?)>>(
      future: _addressesFuture,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return const Center(child: CircularProgressIndicator());
        } else if (snapshot.hasError) {
          return Center(
            child: Text(
              'Failed to load addresses',
              style: Theme.of(context).textTheme.bodyMedium,
            ),
          );
        } else if (!snapshot.hasData || snapshot.data!.isEmpty) {
          return Center(
            child: Text(
              'No addresses found',
              style: Theme.of(context).textTheme.bodyMedium,
            ),
          );
        }

        final addresses = snapshot.data!;

        return ListView.builder(
          itemCount: addresses.length,
          itemBuilder: (context, index) {
            final (address, tweakIdx, amount) = addresses[index];
            final explorerUrl = _explorerUrl(address);

            return Card(
              margin: const EdgeInsets.symmetric(vertical: 6, horizontal: 4),
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(12),
                side: BorderSide(
                  color: Theme.of(context).colorScheme.primary.withOpacity(0.4),
                  width: 1,
                ),
              ),
              color:
                  amount != null
                      ? Theme.of(context).colorScheme.primary.withOpacity(0.1)
                      : Theme.of(context).colorScheme.surface,
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 8,
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    // Address row with abbreviation and buttons
                    Row(
                      children: [
                        Expanded(
                          child: SelectableText(
                            abbreviateAddress(address),
                            style: Theme.of(
                              context,
                            ).textTheme.bodyLarge?.copyWith(letterSpacing: 0.8),
                            maxLines: 1,
                          ),
                        ),
                        const SizedBox(width: 8),

                        // mempool.space link button
                        if (explorerUrl != null)
                          IconButton(
                            tooltip: 'View on mempool.space',
                            icon: const Icon(Icons.open_in_new),
                            color: Theme.of(context).colorScheme.secondary,
                            onPressed: () async {
                              final url = Uri.parse(explorerUrl);
                              await showExplorerConfirmation(context, url);
                            },
                          ),

                        // Copy button
                        IconButton(
                          tooltip: 'Copy address',
                          icon: const Icon(Icons.copy),
                          color: Theme.of(context).colorScheme.primary,
                          onPressed: () {
                            Clipboard.setData(ClipboardData(text: address));
                            ScaffoldMessenger.of(context).showSnackBar(
                              const SnackBar(
                                content: Text('Address copied to clipboard'),
                              ),
                            );
                          },
                        ),

                        // Refresh button
                        IconButton(
                          tooltip: 'Recheck address',
                          icon: const Icon(Icons.refresh),
                          color: Theme.of(context).colorScheme.primary,
                          onPressed: () async {
                            await _refreshAddress(tweakIdx, address);
                          },
                        ),
                      ],
                    ),

                    if (amount != null) ...[
                      const SizedBox(height: 6),
                      Text(
                        formatSats(amount),
                        style: Theme.of(context).textTheme.titleLarge?.copyWith(
                          color: Theme.of(context).colorScheme.primary,
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                    ],
                  ],
                ),
              ),
            );
          },
        );
      },
    );
  }
}
