import 'package:carbine/dashboard.dart';
import 'package:carbine/lib.dart';
import 'package:carbine/scan.dart';
import 'package:carbine/setttings.dart';
import 'package:carbine/sidebar.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/welcome.dart';
import 'package:flutter/material.dart';

class MyApp extends StatefulWidget {
  final List<FederationSelector> initialFederations;
  const MyApp({super.key, required this.initialFederations});

  @override
  State<MyApp> createState() => _MyAppState();
}

class _MyAppState extends State<MyApp> {
  late List<FederationSelector> _feds;
  int _refreshTrigger = 0;
  FederationSelector? _selectedFederation;
  int _currentIndex = 0;

  @override
  void initState() {
    super.initState();
    _feds = widget.initialFederations;

    if (_feds.isNotEmpty) {
      _selectedFederation = _feds.first;
    }
  }

  void _onJoinPressed(FederationSelector fed) {
    _setSelectedFederation(fed);
    _refreshFederations();
  }

  void _setSelectedFederation(FederationSelector fed) {
    setState(() {
      _selectedFederation = fed;
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
    final result = await Navigator.push<FederationSelector>(
      context,
      MaterialPageRoute(builder: (context) => const ScanQRPage()),
    );

    if (result != null) {
      _setSelectedFederation(result);
      _refreshFederations();
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text("Joined ${result.federationName}")),
      );
    } else {
      print('Result is null, not updating federations');
    }
  }

  @override
  Widget build(BuildContext context) {
    Widget bodyContent;

    if (_selectedFederation != null) {
      bodyContent = Dashboard(
        key: ValueKey(_selectedFederation!.federationId),
        fed: _selectedFederation!,
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
                  feds: _feds,
                  onFederationSelected: _setSelectedFederation,
                ),
              ),
              body: SafeArea(child: bodyContent),
            ),
      ),
    );
  }
}
