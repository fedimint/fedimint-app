import 'dart:async';

import 'package:carbine/screens/dashboard.dart';
import 'package:carbine/lib.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/scan.dart';
import 'package:carbine/setttings.dart';
import 'package:carbine/sidebar.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/toast.dart';
import 'package:carbine/utils.dart';
import 'package:carbine/welcome.dart';
import 'package:flutter/material.dart';

final invoicePaidToastVisible = ValueNotifier<bool>(true);

class MyApp extends StatefulWidget {
  final List<(FederationSelector, bool)> initialFederations;
  final bool recoverFederationInviteCodes;
  const MyApp({
    super.key,
    required this.initialFederations,
    required this.recoverFederationInviteCodes,
  });

  @override
  State<MyApp> createState() => _MyAppState();
}

class _MyAppState extends State<MyApp> {
  late List<(FederationSelector, bool)> _feds;
  int _refreshTrigger = 0;
  FederationSelector? _selectedFederation;
  bool? _isRecovering;
  int _currentIndex = 0;

  late Stream<MultimintEvent> events;
  late StreamSubscription<MultimintEvent> _subscription;

  final GlobalKey<NavigatorState> _navigatorKey = ToastService().navigatorKey;

  bool recoverFederations = false;

  @override
  void initState() {
    super.initState();
    initDisplaySetting();
    _feds = widget.initialFederations;

    if (_feds.isNotEmpty) {
      _selectedFederation = _feds.first.$1;
      _isRecovering = _feds.first.$2;
    } else if (_feds.isEmpty && widget.recoverFederationInviteCodes) {
      _rejoinFederations();
    }

    events = subscribeMultimintEvents().asBroadcastStream();
    _subscription = events.listen((event) async {
      if (event is MultimintEvent_Lightning) {
        final ln = event.field0.$2;
        if (ln is LightningEventKind_InvoicePaid) {
          if (!invoicePaidToastVisible.value) {
            AppLogger.instance.info("Request modal visible — skipping toast.");
            return;
          }

          final amountMsats = ln.field0.amountMsats;
          await _handleFundsReceived(
            federationId: event.field0.$1,
            amountMsats: amountMsats,
            icon: Icon(Icons.flash_on, color: Colors.amber),
          );
        }
      } else if (event is MultimintEvent_Log) {
        AppLogger.instance.rustLog(event.field0, event.field1);
      } else if (event is MultimintEvent_Ecash) {
        if (!invoicePaidToastVisible.value) {
          AppLogger.instance.info("Request modal visible — skipping toast.");
          return;
        }
        final amountMsats = event.field0.$2;
        await _handleFundsReceived(
          federationId: event.field0.$1,
          amountMsats: amountMsats,
          icon: Icon(Icons.currency_bitcoin, color: Colors.greenAccent),
        );
      }
    });
  }

  Future<void> _handleFundsReceived({
    required FederationId federationId,
    required BigInt amountMsats,
    required Icon icon,
  }) async {
    final amount = formatBalance(amountMsats, false);
    final federationIdString = await federationIdToString(
      federationId: federationId,
    );

    FederationSelector? selector;
    bool? recovering;

    for (var sel in _feds) {
      final idString = await federationIdToString(
        federationId: sel.$1.federationId,
      );
      if (idString == federationIdString) {
        selector = sel.$1;
        recovering = sel.$2;
        break;
      }
    }

    if (selector == null) return;

    final name = selector.federationName;
    AppLogger.instance.info("$name received $amount");

    ToastService().show(
      message: "$name received $amount",
      duration: const Duration(seconds: 7),
      onTap: () {
        _navigatorKey.currentState?.popUntil((route) => route.isFirst);
        _setSelectedFederation(selector!, recovering!);
      },
      icon: icon,
    );
  }

  Future<void> _rejoinFederations() async {
    setState(() {
      recoverFederations = true;
    });
    await rejoinFromBackupInvites();
    await _refreshFederations();

    if (_feds.isNotEmpty) {
      final first = _feds.first;
      _setSelectedFederation(first.$1, first.$2);
    }

    setState(() {
      recoverFederations = false;
    });
  }

  @override
  void dispose() {
    _subscription.cancel();
    super.dispose();
  }

  void _onJoinPressed(FederationSelector fed, bool recovering) {
    _setSelectedFederation(fed, recovering);
    _refreshFederations();
  }

  void _setSelectedFederation(FederationSelector fed, bool recovering) {
    setState(() {
      _selectedFederation = fed;
      _isRecovering = recovering;
      _currentIndex = 0;
    });
  }

  Future<void> _refreshFederations() async {
    final feds = await federations();
    setState(() {
      _feds = feds;
      _refreshTrigger++;
    });
  }

  void _onScanPressed(BuildContext context) async {
    final result = await Navigator.push<(FederationSelector, bool)>(
      context,
      MaterialPageRoute(
        builder: (context) => ScanQRPage(onPay: _onJoinPressed),
      ),
    );

    if (result != null) {
      _setSelectedFederation(result.$1, result.$2);
      _refreshFederations();
      ToastService().show(
        message: "Joined ${result.$1.federationName}",
        duration: const Duration(seconds: 5),
        onTap: () {},
        icon: Icon(Icons.info),
      );
    } else {
      AppLogger.instance.warn('Scan result is null, not updating federations');
    }
  }

  void _onGettingStarted() {
    setState(() {
      _selectedFederation = null;
      _currentIndex = 0;
    });
  }

  @override
  Widget build(BuildContext context) {
    Widget bodyContent;

    if (_selectedFederation != null) {
      bodyContent = Dashboard(
        key: ValueKey(_selectedFederation!.federationId),
        fed: _selectedFederation!,
        recovering: _isRecovering!,
      );
    } else {
      if (_currentIndex == 1) {
        bodyContent = SettingsScreen(
          onJoin: _onJoinPressed,
          onGettingStarted: _onGettingStarted,
        );
      } else {
        if (recoverFederations) {
          bodyContent = const Center(
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                CircularProgressIndicator(),
                SizedBox(height: 16),
                Text(
                  'Retrieving federation backup from Nostr...',
                  style: TextStyle(fontSize: 16),
                ),
              ],
            ),
          );
        } else {
          bodyContent = WelcomeWidget(onJoin: _onJoinPressed);
        }
      }
    }

    return MaterialApp(
      title: 'Carbine',
      debugShowCheckedModeBanner: false,
      theme: cypherpunkNinjaTheme,
      navigatorKey: _navigatorKey,
      home: Builder(
        builder:
            (innerContext) => Scaffold(
              appBar: AppBar(
                actions: [
                  IconButton(
                    icon: const Icon(Icons.qr_code_scanner),
                    tooltip: 'Scan',
                    onPressed: () => _onScanPressed(innerContext),
                  ),
                  IconButton(
                    icon: const Icon(Icons.settings),
                    tooltip: 'Settings',
                    onPressed: () {
                      setState(() {
                        _currentIndex = 1;
                        _selectedFederation = null;
                      });
                    },
                  ),
                ],
              ),
              drawer: SafeArea(
                child: FederationSidebar(
                  key: ValueKey(_refreshTrigger),
                  initialFederations: _feds,
                  onFederationSelected: _setSelectedFederation,
                ),
              ),
              body: SafeArea(child: bodyContent),
            ),
      ),
    );
  }
}
