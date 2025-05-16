import 'package:carbine/lib.dart';
import 'package:carbine/main.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

class FederationPreview extends StatefulWidget {
  final String federationName;
  final String inviteCode;
  final String? welcomeMessage;
  final String? imageUrl;
  final bool joinable;
  final List<Guardian>? guardians;
  final String network;

  const FederationPreview({
    super.key,
    required this.federationName,
    required this.inviteCode,
    this.welcomeMessage,
    this.imageUrl,
    required this.joinable,
    this.guardians,
    required this.network,
  });

  @override
  State<FederationPreview> createState() => _FederationPreviewState();
}

class _FederationPreviewState extends State<FederationPreview> {
  bool isJoining = false;

  Future<void> _onButtonPressed() async {
    if (widget.joinable) {
      setState(() {
        isJoining = true;
      });
      try {
        final fed = await joinFederation(
          inviteCode: widget.inviteCode,
          recover: false,
        );
        print('Successfully joined federation');
        if (mounted) {
          Navigator.of(context).pop(fed);
        }
      } catch (e) {
        print('Could not join federation $e');
        setState(() {
          isJoining = false;
        });
      }
    } else {
      // TODO: show toast here
      Clipboard.setData(ClipboardData(text: widget.inviteCode));
      print('Invite code copied');
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final totalGuardians =
        widget.guardians != null ? widget.guardians!.length : 0;
    final thresh = threshold(totalGuardians);
    final onlineGuardians =
        widget.guardians != null
            ? widget.guardians!.where((g) => g.version != null).toList()
            : [];
    final isFederationOnline =
        totalGuardians > 0 &&
        onlineGuardians.length >= threshold(totalGuardians);

    return Padding(
      padding: const EdgeInsets.all(16),
      child: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (widget.network.toLowerCase() != 'bitcoin') ...[
              Container(
                margin: const EdgeInsets.only(bottom: 16),
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: Colors.orange.shade100,
                  borderRadius: BorderRadius.circular(8),
                ),
                child: Row(
                  children: [
                    const Icon(Icons.warning, color: Colors.orange),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        'Warning: This is a test network (${widget.network}) and is not worth anything.',
                        style: const TextStyle(color: Colors.orange),
                      ),
                    ),
                  ],
                ),
              ),
            ],

            // Federation image
            Center(
              child: ClipRRect(
                borderRadius: BorderRadius.circular(16),
                child: SizedBox(
                  width: 150,
                  height: 150,
                  child:
                      widget.imageUrl != null
                          ? Image.network(
                            widget.imageUrl!,
                            fit: BoxFit.cover,
                            errorBuilder: (context, error, stackTrace) {
                              return Image.asset(
                                'assets/images/fedimint.png',
                                fit: BoxFit.cover,
                              );
                            },
                          )
                          : Image.asset(
                            'assets/images/fedimint.png',
                            fit: BoxFit.cover,
                          ),
                ),
              ),
            ),

            const SizedBox(height: 16),

            // Federation name
            Text(
              widget.federationName,
              style: theme.textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.bold,
              ),
              textAlign: TextAlign.center,
            ),

            // Welcome message
            if (widget.welcomeMessage != null) ...[
              const SizedBox(height: 12),
              Text(
                widget.welcomeMessage!,
                style: theme.textTheme.bodyMedium,
                textAlign: TextAlign.center,
              ),
            ],

            const SizedBox(height: 24),

            if (isFederationOnline) ...[
              // Join / Copy button
              SizedBox(
                width: double.infinity,
                child: ElevatedButton(
                  onPressed: _onButtonPressed,
                  style: ElevatedButton.styleFrom(
                    backgroundColor: theme.colorScheme.primary,
                    foregroundColor: Colors.black,
                    padding: const EdgeInsets.symmetric(vertical: 16),
                    shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(12),
                    ),
                    textStyle: const TextStyle(
                      fontSize: 16,
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                  child:
                      isJoining
                          ? const SizedBox(
                            height: 20,
                            width: 20,
                            child: CircularProgressIndicator(
                              color: Colors.black,
                              strokeWidth: 2,
                            ),
                          )
                          : Text(
                            widget.joinable
                                ? "Join Federation"
                                : "Copy Invite Code",
                          ),
                ),
              ),

              // Recover button
              if (widget.joinable && !isJoining) ...[
                const SizedBox(height: 12),
                SizedBox(
                  width: double.infinity,
                  child: OutlinedButton.icon(
                    onPressed: () async {
                      setState(() {
                        isJoining = true;
                      });
                      try {
                        final fed = await joinFederation(
                          inviteCode: widget.inviteCode,
                          recover: true,
                        );
                        if (mounted) {
                          Navigator.of(context).pop(fed);
                        }
                      } catch (e) {
                        print('Could not recover federation $e');
                        setState(() {
                          isJoining = false;
                        });
                      }
                    },
                    icon: const Icon(Icons.history),
                    label: const Text('Recover'),
                    style: OutlinedButton.styleFrom(
                      foregroundColor: theme.colorScheme.secondary,
                      side: BorderSide(
                        color: theme.colorScheme.secondary.withOpacity(0.5),
                      ),
                      padding: const EdgeInsets.symmetric(vertical: 16),
                      shape: RoundedRectangleBorder(
                        borderRadius: BorderRadius.circular(12),
                      ),
                      textStyle: const TextStyle(
                        fontSize: 16,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ),
              ],

              // Guardian list
              if (widget.guardians != null && widget.guardians!.isNotEmpty) ...[
                const SizedBox(height: 24),
                Text(
                  'Guardians ($thresh/$totalGuardians federation)',
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.bold,
                  ),
                ),
                const SizedBox(height: 8),
                ListView.builder(
                  shrinkWrap: true,
                  physics: const NeverScrollableScrollPhysics(),
                  itemCount: widget.guardians!.length,
                  itemBuilder: (context, index) {
                    final guardian = widget.guardians![index];
                    final isOnline = guardian.version != null;

                    return ListTile(
                      dense: true,
                      contentPadding: EdgeInsets.zero,
                      leading: Icon(
                        Icons.circle,
                        color: isOnline ? Colors.green : Colors.red,
                        size: 12,
                      ),
                      title: Text(guardian.name),
                      subtitle:
                          isOnline
                              ? Text('Version: ${guardian.version}')
                              : const Text('Offline'),
                    );
                  },
                ),
              ],
            ] else ...[
              const SizedBox(height: 16),
              const Text(
                "This federation is offline, please reach out to the guardian operators.",
                style: TextStyle(fontSize: 16, color: Colors.red),
                textAlign: TextAlign.center,
              ),
            ],
          ],
        ),
      ),
    );
  }
}
