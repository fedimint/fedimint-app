import 'dart:io';

import 'package:carbine/app.dart';
import 'package:carbine/lib.dart';
import 'package:carbine/seed_input.dart';
import 'package:flutter/material.dart';

class CreateWallet extends StatefulWidget {
  final Directory dir;
  const CreateWallet({super.key, required this.dir});

  @override
  State<CreateWallet> createState() => _CreateWalletState();
}

class _CreateWalletState extends State<CreateWallet> {
  bool _isCreating = false;

  Future<void> _handleCreateWallet() async {
    setState(() {
      _isCreating = true;
    });

    try {
      await createNewMultimint(path: widget.dir.path);
      if (mounted) {
        Navigator.of(context).pushReplacement(
          MaterialPageRoute(builder: (_) => MyApp(initialFederations: [])),
        );
      }
    } catch (e) {
      setState(() => _isCreating = false);
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text("Error creating wallet: $e")));
    }
  }

  Future<void> _handleRecoverWallet(List<String> words) async {
    try {
      await createMultimintFromWords(path: widget.dir.path, words: words);
      if (mounted) {
        Navigator.of(context).pushReplacement(
          MaterialPageRoute(builder: (_) => MyApp(initialFederations: [])),
        );
      }
    } catch (e) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text("Error creating wallet: $e")));
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24.0),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              const SizedBox(height: 48),
              Icon(Icons.shield, size: 64, color: theme.colorScheme.primary),
              const SizedBox(height: 24),
              Text(
                'Welcome to Carbine Wallet',
                style: theme.textTheme.headlineSmall?.copyWith(
                  fontWeight: FontWeight.bold,
                  color: theme.colorScheme.primary,
                ),
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: 12),
              Text(
                'Choose how you want to get started.',
                style: theme.textTheme.bodyMedium,
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: 48),

              _WalletOptionCard(
                icon: Icons.fiber_new,
                title: 'Create New Wallet',
                description:
                    'Set up a brand new wallet with a secure seed phrase.',
                onTap: _isCreating ? null : _handleCreateWallet,
                trailing:
                    _isCreating
                        ? const SizedBox(
                          height: 20,
                          width: 20,
                          child: CircularProgressIndicator(
                            strokeWidth: 2,
                            valueColor: AlwaysStoppedAnimation<Color>(
                              Colors.greenAccent,
                            ),
                          ),
                        )
                        : null,
              ),
              const SizedBox(height: 24),

              _WalletOptionCard(
                icon: Icons.settings_backup_restore,
                title: 'Recover Wallet',
                description: 'Restore your wallet using a recovery phrase.',
                onTap:
                    _isCreating
                        ? null
                        : () {
                          Navigator.of(context).push(
                            MaterialPageRoute(
                              builder:
                                  (_) => SeedPhraseInput(
                                    onConfirm: _handleRecoverWallet,
                                  ),
                            ),
                          );
                        },
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _WalletOptionCard extends StatelessWidget {
  final IconData icon;
  final String title;
  final String description;
  final VoidCallback? onTap;
  final Widget? trailing;

  const _WalletOptionCard({
    required this.icon,
    required this.title,
    required this.description,
    required this.onTap,
    this.trailing,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(16),
      child: Ink(
        decoration: BoxDecoration(
          color: theme.colorScheme.surface,
          borderRadius: BorderRadius.circular(16),
          border: Border.all(color: theme.colorScheme.primary.withOpacity(0.2)),
        ),
        padding: const EdgeInsets.all(20),
        child: Row(
          children: [
            Icon(icon, size: 32, color: theme.colorScheme.secondary),
            const SizedBox(width: 16),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    title,
                    style: theme.textTheme.titleLarge?.copyWith(
                      fontWeight: FontWeight.bold,
                      color: theme.colorScheme.primary,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    description,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: Colors.white70,
                    ),
                  ),
                ],
              ),
            ),
            if (trailing != null) ...[const SizedBox(width: 12), trailing!],
          ],
        ),
      ),
    );
  }
}
