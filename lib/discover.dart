import 'package:carbine/fed_preview.dart';
import 'package:carbine/lib.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/nostr.dart';
import 'package:carbine/theme.dart';
import 'package:flutter/material.dart';
import 'package:url_launcher/url_launcher.dart';

// ... existing imports remain the same ...

class Discover extends StatefulWidget {
  final void Function(FederationSelector fed, bool recovering) onJoin;
  const Discover({super.key, required this.onJoin});

  @override
  State<Discover> createState() => _Discover();
}

class _Discover extends State<Discover> {
  late Future<List<PublicFederation>> _futureFeds;
  PublicFederation? _gettingMetadata;

  @override
  void initState() {
    super.initState();
    _futureFeds = listFederationsFromNostr(forceUpdate: false);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(
        title: const Text('Discover Federations'),
        centerTitle: true,
      ),
      body: FutureBuilder<List<PublicFederation>>(
        future: _futureFeds,
        builder: (context, snapshot) {
          if (snapshot.connectionState == ConnectionState.waiting) {
            return const Center(child: CircularProgressIndicator());
          } else if (snapshot.hasError) {
            return Center(
              child: Text(
                "Error: ${snapshot.error}",
                style: TextStyle(color: theme.colorScheme.error),
              ),
            );
          } else if (!snapshot.hasData || snapshot.data!.isEmpty) {
            return const Center(
              child: Text("No public federations available to join"),
            );
          }

          final federations = snapshot.data!;
          return ListView(
            padding: const EdgeInsets.all(16),
            children: [
              _buildHeader(theme),
              const SizedBox(height: 16),
              ...federations.map(
                (federation) => _buildFederationCard(federation, theme),
              ),
              const SizedBox(height: 24),
              _buildObserverLink(theme),
            ],
          );
        },
      ),
    );
  }

  Widget _buildHeader(ThemeData theme) {
    return Row(
      children: [
        Image.asset('assets/images/nostr.png', width: 48, height: 48),
        const SizedBox(width: 12),
        Expanded(
          child: Text(
            "Find new federations to join using this list powered by Nostr.",
            style: theme.textTheme.bodyMedium,
          ),
        ),
      ],
    );
  }

  Widget _buildObserverLink(ThemeData theme) {
    return Center(
      child: GestureDetector(
        onTap: () => launchUrl(Uri.parse("https://observer.fedimint.org/")),
        child: Text(
          "Explore more at observer.fedimint.org",
          style: theme.textTheme.bodyMedium?.copyWith(
            color: theme.colorScheme.secondary,
            decoration: TextDecoration.underline,
          ),
        ),
      ),
    );
  }

  Widget _buildFederationCard(PublicFederation federation, ThemeData theme) {
    return Card(
      color: theme.colorScheme.surface,
      elevation: 2,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            ClipRRect(
              borderRadius: BorderRadius.circular(12),
              child:
                  federation.picture != null && federation.picture!.isNotEmpty
                      ? Image.network(
                        federation.picture!,
                        width: 50,
                        height: 50,
                        fit: BoxFit.cover,
                        errorBuilder:
                            (_, __, ___) => Image.asset(
                              'assets/images/fedimint.png',
                              width: 50,
                              height: 50,
                            ),
                      )
                      : Image.asset(
                        'assets/images/fedimint.png',
                        width: 50,
                        height: 50,
                      ),
            ),
            const SizedBox(width: 16),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    federation.federationName,
                    style: theme.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.bold,
                      color: theme.colorScheme.primary,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    "Network: ${federation.network == 'bitcoin' ? 'mainnet' : federation.network}",
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                  if (federation.about != null &&
                      federation.about!.isNotEmpty) ...[
                    const SizedBox(height: 6),
                    Text(
                      federation.about!,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ],
              ),
            ),
            const SizedBox(width: 12),
            _gettingMetadata == federation
                ? const SizedBox(
                  width: 24,
                  height: 24,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
                : ElevatedButton.icon(
                  style: ElevatedButton.styleFrom(
                    backgroundColor: theme.colorScheme.primary,
                    foregroundColor: Colors.black,
                    shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(12),
                    ),
                    padding: const EdgeInsets.symmetric(
                      horizontal: 14,
                      vertical: 10,
                    ),
                  ),
                  onPressed: () async {
                    setState(() => _gettingMetadata = federation);
                    final meta = await getFederationMeta(
                      inviteCode: federation.inviteCodes.first,
                    );
                    setState(() => _gettingMetadata = null);

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
                      widget.onJoin(fed.$1, fed.$2);
                      if (context.mounted) Navigator.pop(context);
                      ScaffoldMessenger.of(context).showSnackBar(
                        SnackBar(content: Text("Joined ${fed.federationName}")),
                      );
                    }
                  },
                  icon: const Icon(Icons.info_outline, size: 18),
                  label: const Text("Preview"),
                ),
          ],
        ),
      ),
    );
  }
}
