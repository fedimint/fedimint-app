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
  const MyApp({super.key, required this.initialFederations});

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
  int _dashboardReloadTrigger = 0;

  @override
  void initState() {
    super.initState();
    _feds = widget.initialFederations;

    if (_feds.isNotEmpty) {
      _selectedFederation = _feds.first.$1;
      _isRecovering = _feds.first.$2;
    }

    events = subscribeMultimintEvents().asBroadcastStream();
    _subscription = events.listen((event) async {
      if (event.eventKind is MultimintEventKind_Lightning) {
        final ln = event.eventKind as MultimintEventKind_Lightning;
        if (ln.field0 is LightningEventKind_InvoicePaid) {
          if (!invoicePaidToastVisible.value) {
            AppLogger.instance.info("Request modal visible — skipping toast.");
            return;
          }

          final lnEvent = ln.field0 as LightningEventKind_InvoicePaid;
          final amountMsats = lnEvent.field0.amountMsats;
          final amount = formatBalance(amountMsats, false);
          final federationIdString = await federationIdToString(
            federationId: event.federationId,
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

          if (selector == null) {
            return;
          }

          final name = selector.federationName;
          AppLogger.instance.info("$name received $amount");

          setState(() {
            _dashboardReloadTrigger++;
          });

          ToastService().show(
            message: "$name received $amount",
            duration: const Duration(seconds: 7),
            onTap: () {
              _navigatorKey.currentState?.popUntil((route) => route.isFirst);
              _setSelectedFederation(selector!, recovering!);
            },
          );
        }
      }
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

  void _refreshFederations() async {
    final feds = await federations();
    setState(() {
      _feds = feds;
      _refreshTrigger++;
    });
  }

  void _onScanPressed(BuildContext context) async {
    final result = await Navigator.push<(FederationSelector, bool)>(
      context,
      MaterialPageRoute(builder: (context) => const ScanQRPage()),
    );

    if (result != null) {
      _setSelectedFederation(result.$1, result.$2);
      _refreshFederations();
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text("Joined ${result.$1.federationName}")),
      );
    } else {
      AppLogger.instance.warn('Scan result is null, not updating federations');
    }
  }

  @override
  Widget build(BuildContext context) {
    Widget bodyContent;

    if (_selectedFederation != null) {
      bodyContent = Dashboard(
        key: ValueKey(
          '${_selectedFederation!.federationId}--$_dashboardReloadTrigger',
        ),
        fed: _selectedFederation!,
        recovering: _isRecovering!,
      );
    } else {
      if (_currentIndex == 1) {
        bodyContent = SettingsScreen(onJoin: _onJoinPressed);
      } else {
        bodyContent = WelcomeWidget(onJoin: _onJoinPressed);
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
