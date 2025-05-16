import 'package:carbine/discover.dart';
import 'package:carbine/lib.dart';
import 'package:flutter/material.dart';

class SettingsScreen extends StatelessWidget {
  final void Function(FederationSelector fed) onJoin;
  const SettingsScreen({super.key, required this.onJoin});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          _SettingsOption(
            icon: Icons.explore,
            title: "Discover",
            subtitle: "Find new Federations using Nostr",
            onTap: () {
              Navigator.push(
                context,
                MaterialPageRoute(
                  builder: (context) => Discover(onJoin: onJoin),
                ),
              );
            },
          ),
          _SettingsOption(
            icon: Icons.link,
            title: 'Nostr Wallet Connect',
            subtitle: 'Connect to NWC-compatible apps',
            onTap: () {
              // Handle NWC tap
            },
          ),
          const SizedBox(height: 12),
          _SettingsOption(
            icon: Icons.flash_on,
            title: 'Lightning Address',
            subtitle: 'Set or update your LN address',
            onTap: () {
              // Handle Lightning Address tap
            },
          ),
          const SizedBox(height: 12),
          _SettingsOption(
            icon: Icons.vpn_key,
            title: 'Mnemonic',
            subtitle: 'View or export your seed phrase',
            onTap: () async {
              final words = await getMnemonic();
              print("Mnemonic: $words");
            },
          ),
        ],
      ),
    );
  }
}

class _SettingsOption extends StatelessWidget {
  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback onTap;

  const _SettingsOption({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      elevation: 2,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(16),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Row(
            children: [
              Icon(icon, size: 28, color: theme.colorScheme.primary),
              const SizedBox(width: 16),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      title,
                      style: theme.textTheme.titleMedium?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      subtitle,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
              const Icon(Icons.chevron_right),
            ],
          ),
        ),
      ),
    );
  }
}
