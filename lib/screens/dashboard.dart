import 'dart:async';

import 'package:carbine/recovery_progress.dart';
import 'package:carbine/utils.dart';
import 'package:carbine/widgets/addresses.dart';
import 'package:flutter/material.dart';
import 'package:flutter_speed_dial/flutter_speed_dial.dart';

import 'package:carbine/lib.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/number_pad.dart';
import 'package:carbine/payment_selector.dart';
import 'package:carbine/onchain_receive.dart';
import 'package:carbine/scan.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/models.dart';

import 'package:carbine/widgets/dashboard_header.dart';
import 'package:carbine/widgets/dashboard_balance.dart';
import 'package:carbine/widgets/transactions_list.dart';

class Dashboard extends StatefulWidget {
  final FederationSelector fed;
  final bool recovering;

  const Dashboard({super.key, required this.fed, required this.recovering});

  @override
  _DashboardState createState() => _DashboardState();
}

class _DashboardState extends State<Dashboard> {
  BigInt? balanceMsats;
  bool isLoadingBalance = true;
  bool showMsats = false;
  late bool recovering;
  double _recoveryProgress = 0.0;
  PaymentType _selectedPaymentType = PaymentType.lightning;
  VoidCallback? _pendingAction;
  VoidCallback? _refreshTransactionsList;
  double? _btcPrice;

  late Stream<MultimintEvent> events;
  late StreamSubscription<MultimintEvent> _subscription;

  @override
  void initState() {
    super.initState();
    recovering = widget.recovering;
    _loadBalance();
    _loadBtcPrice();

    events = subscribeMultimintEvents().asBroadcastStream();
    _subscription = events.listen((event) async {
      if (event is MultimintEvent_Lightning) {
        final ln = event.field0.$2;
        if (ln is LightningEventKind_InvoicePaid) {
          final federationIdString = await federationIdToString(
            federationId: event.field0.$1,
          );
          final selectorIdString = await federationIdToString(
            federationId: widget.fed.federationId,
          );
          if (federationIdString == selectorIdString) {
            _loadBalance();
          }
        }
      } else if (event is MultimintEvent_RecoveryDone) {
        final recoveredFedId = event.field0;
        final currFederationId = await federationIdToString(federationId: widget.fed.federationId);
        if (currFederationId == recoveredFedId) {
          setState(() => recovering = false);
          _loadBalance();
        }
      } else if (event is MultimintEvent_Ecash) {
        final federationIdString = await federationIdToString(federationId: event.field0.$1);
        final selectorIdString = await federationIdToString(federationId: widget.fed.federationId);
        if (federationIdString == selectorIdString) {
          _loadBalance();
          _selectedPaymentType = PaymentType.ecash;
        }
      }
    });
  }

  @override
  void dispose() {
    super.dispose();
    _subscription.cancel();
  }

  void _scheduleAction(VoidCallback action) {
    setState(() => _pendingAction = action);
  }

  Future<void> _loadBalance() async {
    if (!mounted || recovering) return;
    final bal = await balance(federationId: widget.fed.federationId);
    setState(() {
      balanceMsats = bal;
      isLoadingBalance = false;
    });
  }

  Future<void> _loadBtcPrice() async {
    final price = await fetchBtcPrice();
    if (price != null) {
      setState(() {
        _btcPrice = price.toDouble();
      });
    }
  }

  void _refreshTransactions() {
    _refreshTransactionsList?.call();
  }

  void _onSendPressed() async {
    if (_selectedPaymentType == PaymentType.lightning) {
      await showCarbineModalBottomSheet(
        context: context,
        child: PaymentMethodSelector(fed: widget.fed),
      );
    } else if (_selectedPaymentType == PaymentType.ecash ||
        _selectedPaymentType == PaymentType.onchain) {
      await Navigator.push(
        context,
        MaterialPageRoute(
          builder:
              (_) => NumberPad(
                fed: widget.fed,
                paymentType: _selectedPaymentType,
                btcPrice: _btcPrice,
                onWithdrawCompleted:
                    _selectedPaymentType == PaymentType.onchain
                        ? _refreshTransactions
                        : null,
              ),
        ),
      );
    }
    _loadBalance();
  }

  void _onReceivePressed() async {
    if (_selectedPaymentType == PaymentType.lightning) {
      await Navigator.push(
        context,
        MaterialPageRoute(
          builder:
              (_) => NumberPad(
                fed: widget.fed,
                paymentType: _selectedPaymentType,
                btcPrice: _btcPrice,
                onWithdrawCompleted: null,
              ),
        ),
      );
    } else if (_selectedPaymentType == PaymentType.onchain) {
      await showCarbineModalBottomSheet(
        context: context,
        child: OnChainReceiveContent(fed: widget.fed),
        heightFactor: 0.33,
      );
    } else if (_selectedPaymentType == PaymentType.ecash) {
      await Navigator.push(
        context,
        MaterialPageRoute(builder: (_) => ScanQRPage(selectedFed: widget.fed, paymentType: _selectedPaymentType)),
      );
    }
    _loadBalance();
  }

  Future<void> _loadProgress(PaymentType paymentType) async {
    if (recovering) {
      final progress = await getModuleRecoveryProgress(
        federationId: widget.fed.federationId,
        moduleId: getModuleIdForPaymentType(paymentType),
      );

      if (progress.$2 > 0) {
        double rawProgress = progress.$1.toDouble() / progress.$2.toDouble();
        setState(() => _recoveryProgress = rawProgress.clamp(0.0, 1.0));
      }

      AppLogger.instance.info(
        "$_selectedPaymentType progress: $_recoveryProgress complete: ${progress.$1} total: ${progress.$2}",
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final name = widget.fed.federationName;

    return Scaffold(
      floatingActionButton:
          recovering
              ? null
              : SpeedDial(
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
                  if (balanceMsats != null && balanceMsats! > BigInt.zero) ...[
                    SpeedDialChild(
                      child: const Icon(Icons.upload),
                      label: 'Send',
                      backgroundColor: Colors.blue,
                      onTap: () => _scheduleAction(_onSendPressed),
                    ),
                  ],
                ],
              ),
      body: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          children: [
            const SizedBox(height: 32),
            DashboardHeader(name: name, network: widget.fed.network),
            const SizedBox(height: 48),
            DashboardBalance(
              balanceMsats: balanceMsats,
              isLoading: isLoadingBalance,
              recovering: recovering,
              showMsats: showMsats,
              onToggle: () => setState(() => showMsats = !showMsats),
              btcPrice: _btcPrice,
            ),
            const SizedBox(height: 48),
            if (recovering)...[
              RecoveryStatus(
                key: ValueKey(_selectedPaymentType),
                paymentType: _selectedPaymentType,
                fed: widget.fed,
                initialProgress: _recoveryProgress,
              )
            ] else...[
              Expanded(
                child: DefaultTabController(
                  length: _selectedPaymentType == PaymentType.onchain ? 2 : 1,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      TabBar(
                        indicatorColor: Theme.of(context).colorScheme.primary,
                        labelColor: Theme.of(context).colorScheme.primary,
                        unselectedLabelColor: Colors.grey,
                        tabs: [
                          const Tab(text: 'Recent Transactions'),
                          if (_selectedPaymentType == PaymentType.onchain)
                            const Tab(text: 'Addresses'),
                        ],
                      ),
                      const SizedBox(height: 16),
                      Expanded(
                        child: TabBarView(
                          children: [
                            TransactionsList(
                              key: ValueKey(balanceMsats),
                              fed: widget.fed,
                              selectedPaymentType: _selectedPaymentType,
                              recovering: recovering,
                              onClaimed: _loadBalance,
                              onWithdrawCompleted: _refreshTransactions,
                              onRefreshRequested: (refreshCallback) {
                                _refreshTransactionsList = refreshCallback;
                              },
                            ),
                            if (_selectedPaymentType == PaymentType.onchain)
                              OnchainAddressesList(fed: widget.fed, updateBalance: _loadBalance),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ]
          ],
        ),
      ),
      bottomNavigationBar: BottomNavigationBar(
        currentIndex: _selectedPaymentType.index,
        onTap: (index) async {
          await _loadProgress(PaymentType.values[index]);
          setState(() => _selectedPaymentType = PaymentType.values[index]);
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
