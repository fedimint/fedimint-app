import 'package:carbine/lib.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/number_pad.dart';
import 'package:carbine/payment_selector.dart';
import 'package:carbine/onchain_receive.dart';
import 'package:carbine/scan.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/refund.dart';
import 'package:carbine/utils.dart';
import 'package:flutter/material.dart';
import 'package:flutter_speed_dial/flutter_speed_dial.dart';
import 'package:intl/intl.dart';
import 'dart:async';

class Dashboard extends StatefulWidget {
  final FederationSelector fed;
  final bool recovering;

  const Dashboard({super.key, required this.fed, required this.recovering});

  @override
  State<Dashboard> createState() => _DashboardState();
}

enum PaymentType { lightning, onchain, ecash }

class _DashboardState extends State<Dashboard> {
  BigInt? balanceMsats;
  bool isLoadingBalance = true;
  bool isLoadingTransactions = true;
  final List<Transaction> _transactions = [];
  bool showMsats = false;

  Transaction? _lastTransaction;
  bool _hasMore = true;
  bool _isFetchingMore = false;
  final ScrollController _scrollController = ScrollController();

  PaymentType _selectedPaymentType = PaymentType.lightning;

  VoidCallback? _pendingAction;

  late bool recovering;
  late Stream<DepositEvent> depositEvents;
  late StreamSubscription<DepositEvent> _claimSubscription;
  late StreamSubscription<DepositEvent> _depositSubscription;
  final Map<String, DepositEvent> _depositMap = {};

  @override
  void initState() {
    super.initState();
    setState(() {
      recovering = widget.recovering;
    });
    _scrollController.addListener(_onScroll);
    _loadBalance();
    _loadTransactions();
    depositEvents =
        subscribeDeposits(
          federationId: widget.fed.federationId,
        ).asBroadcastStream();
    _claimSubscription = depositEvents.listen((event) {
      if (event is DepositEvent_Claimed) {
        if (!mounted) return;
        _loadBalance();
        // this timeout is necessary to ensure the claimed on-chain deposit
        // is in the operation log
        Timer(const Duration(milliseconds: 100), () {
          if (!mounted) return;
          _loadTransactions();
        });
      }
    });

    _depositSubscription = depositEvents.listen((event) {
      String txid;
      switch (event) {
        case DepositEvent_Mempool(field0: final mempoolEvt):
          txid = mempoolEvt.txid;
          break;
        case DepositEvent_AwaitingConfs(field0: final awaitEvt):
          txid = awaitEvt.txid;
          break;
        case DepositEvent_Confirmed(field0: final confirmedEvt):
          txid = confirmedEvt.txid;
          break;
        case DepositEvent_Claimed(field0: final claimedEvt):
          txid = claimedEvt.txid;
          break;
      }
      setState(() {
        _depositMap[txid] = event;
      });
    });

    if (recovering) {
      _loadFederation();
    }
  }

  @override
  void dispose() {
    _scrollController.dispose();
    _depositSubscription.cancel();
    _claimSubscription.cancel();

    super.dispose();
  }

  void _scheduleAction(VoidCallback action) {
    setState(() {
      _pendingAction = action;
    });
  }

  void _onScroll() {
    if (_scrollController.position.pixels >=
            _scrollController.position.maxScrollExtent - 100 &&
        !_isFetchingMore &&
        _hasMore) {
      _loadTransactions(loadMore: true);
    }
  }

  List<String> _getKindsForSelectedPaymentType() {
    switch (_selectedPaymentType) {
      case PaymentType.lightning:
        return ['ln', 'lnv2'];
      case PaymentType.onchain:
        return ['wallet'];
      case PaymentType.ecash:
        return ['mint'];
    }
  }

  String _getNoTransactionsMessage() {
    switch (_selectedPaymentType) {
      case PaymentType.lightning:
        return "No lightning transactions yet";
      case PaymentType.onchain:
        return "No onchain transactions yet";
      case PaymentType.ecash:
        return "No ecash transactions yet";
    }
  }

  Future<void> _loadFederation() async {
    await waitForRecovery(inviteCode: widget.fed.inviteCode);
    setState(() {
      recovering = false;
    });

    _loadBalance();
    _loadTransactions();
  }

  Future<void> _loadBalance() async {
    if (!mounted) return;
    if (!recovering) {
      final bal = await balance(federationId: widget.fed.federationId);
      setState(() {
        balanceMsats = bal;
        isLoadingBalance = false;
      });
    }
  }

  Future<void> _loadTransactions({bool loadMore = false}) async {
    if (!recovering) {
      if (_isFetchingMore) return;
      _isFetchingMore = true;

      if (!loadMore) {
        if (mounted) {
          setState(() {
            isLoadingTransactions = true;
            _transactions.clear();
            _hasMore = true;
            _lastTransaction = null;
          });
        }
      }

      final newTxs = await transactions(
        federationId: widget.fed.federationId,
        timestamp: loadMore ? _lastTransaction?.timestamp : null,
        operationId: loadMore ? _lastTransaction?.operationId : null,
        modules: _getKindsForSelectedPaymentType(),
      );

      if (!mounted) return;
      setState(() {
        _transactions.addAll(newTxs);
        if (newTxs.length < 10) {
          _hasMore = false;
        }
        if (newTxs.isNotEmpty) {
          _lastTransaction = newTxs.last;
        }
        isLoadingTransactions = false;
        _isFetchingMore = false;
      });
    }
  }

  void _onSendPressed() async {
    if (_selectedPaymentType == PaymentType.lightning) {
      await showCarbineModalBottomSheet(
        context: context,
        child: PaymentMethodSelector(fed: widget.fed),
      );
    } else if (_selectedPaymentType == PaymentType.ecash) {
      await Navigator.push(
        context,
        MaterialPageRoute(
          builder:
              (context) =>
                  NumberPad(fed: widget.fed, paymentType: _selectedPaymentType),
        ),
      );
    }
    _loadBalance();
    _loadTransactions();
  }

  void _onReceivePressed() async {
    if (_selectedPaymentType == PaymentType.lightning) {
      await Navigator.push(
        context,
        MaterialPageRoute(
          builder:
              (context) =>
                  NumberPad(fed: widget.fed, paymentType: _selectedPaymentType),
        ),
      );
    } else if (_selectedPaymentType == PaymentType.onchain) {
      await Navigator.push(
        context,
        MaterialPageRoute(
          builder: (context) => OnChainReceive(fed: widget.fed),
        ),
      );
    } else if (_selectedPaymentType == PaymentType.ecash) {
      await Navigator.push(
        context,
        MaterialPageRoute(
          builder: (context) => ScanQRPage(selectedFed: widget.fed),
        ),
      );
    }

    _loadBalance();
    _loadTransactions();
  }

  void _onRefundPressed() async {
    await Navigator.push<bool>(
      context,
      MaterialPageRoute(
        builder:
            (_) => RefundConfirmationPage(
              fed: widget.fed,
              balanceMsats: balanceMsats!,
            ),
      ),
    );

    _loadBalance();
    _loadTransactions();
  }

  @override
  Widget build(BuildContext context) {
    final name = widget.fed.federationName;
    final pendingCount =
        (_selectedPaymentType == PaymentType.onchain) ? _depositMap.length : 0;

    int confirmedDisplayCount;
    if (recovering) {
      confirmedDisplayCount = 0;
    } else if (isLoadingTransactions) {
      confirmedDisplayCount = 1;
    } else if (_transactions.isEmpty) {
      confirmedDisplayCount = 1;
    } else {
      confirmedDisplayCount = _transactions.length + (_hasMore ? 1 : 0);
    }

    // Number of fixed, static widgets before the dynamic lists:
    //   0: SizedBox(height:32)
    //   1: ShaderMask(...)
    //   2: SizedBox(height:48)
    //   3: Balance / Recovering widget
    //   4: SizedBox(height:48)
    //   5: Align(Text("Recent Transactions"))
    const int staticHeaderCount = 6;

    final totalCount = staticHeaderCount + pendingCount + confirmedDisplayCount;

    return Scaffold(
      floatingActionButton: SpeedDial(
        icon: Icons.add,
        activeIcon: Icons.close,
        backgroundColor: Theme.of(context).colorScheme.primary,
        foregroundColor: Colors.white,
        onClose: () async {
          if (_pendingAction != null) {
            await Future.delayed(const Duration(milliseconds: 200));
            WidgetsBinding.instance.addPostFrameCallback((_) {
              _pendingAction!();
              _pendingAction = null;
            });
          }
        },
        children: [
          SpeedDialChild(
            child: const Icon(Icons.download),
            label: 'Receive',
            backgroundColor: Colors.green,
            onTap: () => _scheduleAction(_onReceivePressed),
          ),
          if (balanceMsats != null && balanceMsats! > BigInt.zero)
            if (_selectedPaymentType == PaymentType.onchain)
              SpeedDialChild(
                child: const Icon(Icons.reply),
                label: 'Refund',
                backgroundColor: Colors.orange,
                onTap: () => _scheduleAction(_onRefundPressed),
              )
            else
              SpeedDialChild(
                child: const Icon(Icons.upload),
                label: 'Send',
                backgroundColor: Colors.blue,
                onTap: () => _scheduleAction(_onSendPressed),
              ),
        ],
      ),
      body: ListView.builder(
        padding: const EdgeInsets.all(24),
        itemCount: totalCount,
        itemBuilder: (context, index) {
          if (index < staticHeaderCount) {
            switch (index) {
              case 0:
                return const SizedBox(height: 32);
              case 1:
                return ShaderMask(
                  shaderCallback:
                      (bounds) => LinearGradient(
                        colors: [
                          Theme.of(context).colorScheme.primary,
                          Theme.of(context).colorScheme.secondary,
                        ],
                        begin: Alignment.topLeft,
                        end: Alignment.bottomRight,
                      ).createShader(
                        Rect.fromLTWH(0, 0, bounds.width, bounds.height),
                      ),
                  child: Column(
                    children: [
                      Text(
                        name.toUpperCase(),
                        style: Theme.of(
                          context,
                        ).textTheme.headlineMedium?.copyWith(
                          fontWeight: FontWeight.w900,
                          letterSpacing: 2,
                          color: Colors.white,
                          shadows: [
                            Shadow(
                              blurRadius: 10,
                              color: Theme.of(
                                context,
                              ).colorScheme.primary.withOpacity(0.5),
                              offset: const Offset(0, 2),
                            ),
                          ],
                        ),
                        textAlign: TextAlign.center,
                      ),
                      if (widget.fed.network != null &&
                          widget.fed.network!.toLowerCase() != 'bitcoin')
                        Padding(
                          padding: const EdgeInsets.only(top: 8.0),
                          child: Text(
                            "This is a test network and is not worth anything.",
                            style: Theme.of(
                              context,
                            ).textTheme.bodySmall?.copyWith(
                              color: Colors.amberAccent,
                              fontStyle: FontStyle.italic,
                            ),
                            textAlign: TextAlign.center,
                          ),
                        ),
                    ],
                  ),
                );
              case 2:
                return const SizedBox(height: 48);
              case 3:
                if (!recovering) {
                  if (isLoadingBalance) {
                    return const Center(child: CircularProgressIndicator());
                  } else {
                    return Center(
                      child: GestureDetector(
                        onTap: () => setState(() => showMsats = !showMsats),
                        child: Text(
                          formatBalance(balanceMsats, showMsats),
                          style: Theme.of(
                            context,
                          ).textTheme.displayLarge?.copyWith(
                            color: Theme.of(context).colorScheme.primary,
                            fontWeight: FontWeight.bold,
                          ),
                          textAlign: TextAlign.center,
                        ),
                      ),
                    );
                  }
                } else {
                  return Center(
                    child: Text(
                      "Recovering...",
                      style: Theme.of(context).textTheme.displaySmall?.copyWith(
                        color: Theme.of(context).colorScheme.primary,
                        fontWeight: FontWeight.bold,
                      ),
                      textAlign: TextAlign.center,
                    ),
                  );
                }
              case 4:
                return const SizedBox(height: 48);
              case 5:
                return Align(
                  alignment: Alignment.centerLeft,
                  child: Text(
                    "Recent Transactions",
                    style: Theme.of(context).textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                );
            }
          }

          final adjustedIndex = index - staticHeaderCount;

          if (_selectedPaymentType == PaymentType.onchain &&
              adjustedIndex < pendingCount) {
            final events =
                _depositMap.values.toList()..sort((a, b) {
                  final aMempool = a is DepositEvent_Mempool;
                  final bMempool = b is DepositEvent_Mempool;
                  if (aMempool && !bMempool) return -1;
                  if (!aMempool && bMempool) return 1;
                  final BigInt na =
                      a is DepositEvent_AwaitingConfs
                          ? a.field0.needed
                          : BigInt.zero;
                  final BigInt nb =
                      b is DepositEvent_AwaitingConfs
                          ? b.field0.needed
                          : BigInt.zero;
                  return nb.compareTo(na);
                });
            final event = events[adjustedIndex];

            String msg;
            BigInt amount;
            switch (event) {
              case DepositEvent_Mempool(field0: final e):
                msg = 'Tx in mempool';
                amount = e.amount;
                break;
              case DepositEvent_AwaitingConfs(field0: final e):
                msg =
                    'Tx included in block ${e.blockHeight}. Remaining confs: ${e.needed}';
                amount = e.amount;
                break;
              case DepositEvent_Confirmed(field0: final e):
                msg = 'Tx confirmed, claiming ecash';
                amount = e.amount;
                break;
              case DepositEvent_Claimed():
                return const SizedBox.shrink();
            }

            final formattedAmount = formatBalance(amount, false);
            final amountStyle = TextStyle(
              fontWeight: FontWeight.bold,
              color: Colors.greenAccent,
            );

            return Card(
              elevation: 4,
              margin: const EdgeInsets.symmetric(vertical: 6),
              color: Theme.of(context).colorScheme.surface,
              child: ListTile(
                leading: CircleAvatar(
                  backgroundColor: Colors.greenAccent.withOpacity(0.1),
                  child: Icon(Icons.link, color: Colors.yellowAccent),
                ),
                title: Text(
                  "Pending Receive",
                  style: Theme.of(context).textTheme.bodyMedium,
                ),
                subtitle: Text(
                  msg,
                  style: Theme.of(context).textTheme.bodyMedium,
                ),
                trailing: Text(formattedAmount, style: amountStyle),
              ),
            );
          }

          final confirmedStartIndex = pendingCount;
          final confirmIndex = adjustedIndex - confirmedStartIndex;

          if (!recovering && confirmIndex == 0) {
            if (isLoadingTransactions) {
              return const Center(child: CircularProgressIndicator());
            } else if (_transactions.isEmpty && pendingCount == 0) {
              return Center(child: Text(_getNoTransactionsMessage()));
            }
          }

          if (!recovering &&
              !isLoadingTransactions &&
              _transactions.isNotEmpty) {
            if (confirmIndex < _transactions.length) {
              final tx = _transactions[confirmIndex];
              final isIncoming = tx.received;
              final date = DateTime.fromMillisecondsSinceEpoch(
                tx.timestamp.toInt(),
              );
              final formattedDate = DateFormat.yMMMd().add_jm().format(date);
              final formattedAmount = formatBalance(tx.amount, false);

              IconData moduleIcon;
              switch (tx.module) {
                case 'ln':
                case 'lnv2':
                  moduleIcon = Icons.flash_on;
                  break;
                case 'wallet':
                  moduleIcon = Icons.link;
                  break;
                case 'mint':
                  moduleIcon = Icons.currency_bitcoin;
                  break;
                default:
                  moduleIcon = Icons.help_outline;
              }

              final amountStyle = TextStyle(
                fontWeight: FontWeight.bold,
                color: isIncoming ? Colors.greenAccent : Colors.redAccent,
              );

              return Card(
                elevation: 4,
                margin: const EdgeInsets.symmetric(vertical: 6),
                color: Theme.of(context).colorScheme.surface,
                child: ListTile(
                  leading: CircleAvatar(
                    backgroundColor:
                        isIncoming
                            ? Colors.greenAccent.withOpacity(0.1)
                            : Colors.redAccent.withOpacity(0.1),
                    child: Icon(
                      moduleIcon,
                      color: isIncoming ? Colors.greenAccent : Colors.redAccent,
                    ),
                  ),
                  title: Text(
                    isIncoming ? "Received" : "Sent",
                    style: Theme.of(context).textTheme.bodyMedium,
                  ),
                  subtitle: Text(
                    formattedDate,
                    style: Theme.of(context).textTheme.bodyMedium,
                  ),
                  trailing: Text(formattedAmount, style: amountStyle),
                ),
              );
            }

            if (_hasMore) {
              return const Padding(
                padding: EdgeInsets.symmetric(vertical: 12.0),
                child: Center(child: CircularProgressIndicator()),
              );
            }
          }

          return const SizedBox.shrink();
        },
      ),
      bottomNavigationBar: BottomNavigationBar(
        currentIndex: _selectedPaymentType.index,
        onTap: (index) {
          setState(() {
            _selectedPaymentType = PaymentType.values[index];
          });
          _loadTransactions();
        },
        selectedItemColor: Theme.of(context).colorScheme.primary,
        unselectedItemColor: Colors.grey,
        items: const [
          BottomNavigationBarItem(
            icon: Icon(Icons.flash_on),
            label: 'Lightning',
          ),
          BottomNavigationBarItem(icon: Icon(Icons.link), label: 'Onchain'),
          BottomNavigationBarItem(
            icon: Icon(Icons.currency_bitcoin),
            label: 'Ecash',
          ),
        ],
      ),
    );
  }
}
