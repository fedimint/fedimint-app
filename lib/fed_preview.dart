import 'package:carbine/lib.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/utils.dart';
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

  @override
  void initState() {
    super.initState();
  }

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
        AppLogger.instance.info('Successfully joined federation');

        if (mounted) {
          Navigator.of(context).pop((fed, false));
        }

        // backup the federation's invite codes as a replaceable event to Nostr
        final newFeds = await federations();
        final inviteCodes = newFeds.map((f) => f.$1.inviteCode).toList();
        backupInviteCodes(inviteCodes: inviteCodes);
      } catch (e) {
        AppLogger.instance.error('Could not join federation $e');
        setState(() {
          isJoining = false;
        });
      }
    } else {
      Clipboard.setData(ClipboardData(text: widget.inviteCode));
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final totalGuardians = widget.guardians?.length ?? 0;
    final thresh = threshold(totalGuardians);
    final onlineGuardians =
        widget.guardians?.where((g) => g.version != null).toList() ?? [];
    final isFederationOnline =
        totalGuardians > 0 && onlineGuardians.length >= thresh;

    return DefaultTabController(
      length: 2,
      child: Padding(
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

              Text(
                widget.federationName,
                style: theme.textTheme.headlineSmall?.copyWith(
                  fontWeight: FontWeight.bold,
                ),
                textAlign: TextAlign.center,
              ),

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
                            Navigator.of(context).pop((fed, true));
                          }
                        } catch (e) {
                          AppLogger.instance.error(
                            'Could not recover federation $e',
                          );
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

                const SizedBox(height: 24),
                TabBar(
                  labelColor: theme.colorScheme.primary,
                  unselectedLabelColor: Colors.grey,
                  indicatorColor: theme.colorScheme.primary,
                  tabs: [
                    Tab(text: 'Guardians ($thresh/$totalGuardians federation)'),
                    Tab(text: 'UTXOs'),
                  ],
                ),
                SizedBox(
                  height: 300,
                  child: TabBarView(
                    children: [
                      _buildGuardianList(thresh, totalGuardians),
                      FederationUtxoList(
                        invite: widget.inviteCode,
                        network: widget.network,
                      ),
                    ],
                  ),
                ),
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
      ),
    );
  }

  Widget _buildGuardianList(int thresh, int total) {
    return widget.guardians != null && widget.guardians!.isNotEmpty
        ? ListView.builder(
          padding: const EdgeInsets.only(top: 8),
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
        )
        : const Center(child: Text("No guardians available."));
  }
}

class FederationUtxoList extends StatefulWidget {
  final String invite;
  final String network;

  const FederationUtxoList({
    super.key,
    required this.invite,
    required this.network,
  });

  @override
  State<FederationUtxoList> createState() => _FederationUtxoListState();
}

class _FederationUtxoListState extends State<FederationUtxoList> {
  List<Utxo>? utxos;

  @override
  void initState() {
    super.initState();
    _loadWalletSummary();
  }

  Future<void> _loadWalletSummary() async {
    final summary = await walletSummary(invite: widget.invite);
    setState(() {
      utxos = summary;
    });
  }

  String? _explorerUrl(String txid) {
    switch (widget.network) {
      case 'bitcoin':
        return 'https://mempool.space/tx/$txid';
      case 'signet':
        return 'https://mutinynet.com/tx/$txid';
      default:
        return null;
    }
  }

  // Abbreviate the txid with the middle replaced by "..."
  String abbreviateTxid(String txid, {int headLength = 8, int tailLength = 8}) {
    if (txid.length <= headLength + tailLength) return txid;
    final head = txid.substring(0, headLength);
    final tail = txid.substring(txid.length - tailLength);
    return '$head...$tail';
  }

  @override
  Widget build(BuildContext context) {
    if (utxos == null) {
      return const Center(child: CircularProgressIndicator());
    }

    if (utxos!.isEmpty) {
      return const Center(child: Text("No UTXOs found."));
    }

    return ListView.separated(
      padding: const EdgeInsets.all(12),
      itemCount: utxos!.length,
      separatorBuilder: (_, __) => const SizedBox(height: 10),
      itemBuilder: (context, index) {
        final utxo = utxos![index];
        final explorerUrl = _explorerUrl(utxo.txid);
        final abbreviatedTxid = abbreviateTxid(utxo.txid);
        final txidLabel = "$abbreviatedTxid:${utxo.index}";

        return Container(
          decoration: BoxDecoration(
            color: const Color(0xFF1A1A1A),
            borderRadius: BorderRadius.circular(12),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // Row with TxID:index left, explorer link right
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Expanded(
                    child: Text(
                      txidLabel,
                      style: const TextStyle(
                        fontFamily: 'monospace',
                        fontSize: 14,
                        color: Colors.greenAccent,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ),

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
                ],
              ),

              const SizedBox(height: 8),

              // Amount below
              Text(
                formatBalance(utxo.amount, false),
                style: Theme.of(context).textTheme.bodyLarge?.copyWith(
                  fontWeight: FontWeight.bold,
                  fontSize: 16,
                  color: Colors.greenAccent,
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
