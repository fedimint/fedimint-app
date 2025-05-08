import 'package:carbine/dashboard.dart';
import 'package:carbine/frb_generated.dart';
import 'package:carbine/scan.dart';
import 'package:carbine/lib.dart';
import 'package:carbine/setttings.dart';
import 'package:carbine/sidebar.dart';
import 'package:carbine/theme.dart';
import 'package:carbine/welcome.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:path_provider/path_provider.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final dir = await getApplicationDocumentsDirectory();
  await initMultimint(path: dir.path);
  runApp(const MyApp());
}

int threshold(int totalPeers) {
  final maxEvil = (totalPeers - 1) ~/ 3;
  return totalPeers - maxEvil;
}

String formatBalance(BigInt? msats, bool showMsats) {
  if (msats == null) return showMsats ? '0 msats' : '0 sats';

  if (showMsats) {
    final formatter = NumberFormat('#,##0', 'en_US');
    var formatted = formatter.format(msats.toInt());
    formatted = formatted.replaceAll(',', ' ');
    return '$formatted msats';
  } else {
    final sats = msats ~/ BigInt.from(1000);
    final formatter = NumberFormat('#,##0', 'en_US');
    var formatted = formatter.format(sats.toInt());
    return '$formatted sats';
  }
}

class MyApp extends StatefulWidget {
  const MyApp({super.key});

  @override
  State<MyApp> createState() => _MyAppState();
}

class _MyAppState extends State<MyApp> {
  late Future<List<FederationSelector>> _federationFuture;
  int _refreshTrigger = 0;
  FederationSelector? _selectedFederation;
  int _currentIndex = 0;
  bool _initialLoadComplete = false;

  @override
  void initState() {
    super.initState();

    // Add a delay so splash screen is visible for a bit
    Future.delayed(const Duration(seconds: 2), () async {
      final feds = await federations();

      if (mounted) {
        setState(() {
          if (feds.isNotEmpty) {
            _selectedFederation = feds.first;
          }
          _initialLoadComplete = true;
        });
      }
    });

    _refreshFederations();
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

  void _refreshFederations() {
    setState(() {
      _federationFuture = federations();
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

  void _onNavBarTapped(int index, BuildContext context) async {
    setState(() {
      _currentIndex = index;
      if (index == 1) {
        _selectedFederation = null;
      }
    });

    if (index == 0) {
      _onScanPressed(context);
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
      if (!_initialLoadComplete) {
        // Show splash screen
        bodyContent = Center(
          child: ClipRRect(
            borderRadius: BorderRadius.circular(20), // Adjust radius as needed
            child: Image.asset(
              'assets/images/fedimint.png',
              width: 200, // Optional: constrain width
              height: 200, // Optional: constrain height
              fit: BoxFit.cover,
            ),
          ),
        );
      } else {
        if (_currentIndex == 1) {
          // Show settings screen after initial load
          bodyContent = SettingsScreen(onJoin: _onJoinPressed);
        } else {
          bodyContent = WelcomeWidget(onJoin: _onJoinPressed);
        }
      }
    }

    return MaterialApp(
      title: 'Carbine',
      debugShowCheckedModeBanner: false,
      theme: cypherpunkNinjaTheme,
      home: Builder(
        builder: (innerContext) => Scaffold(
          appBar: AppBar(),
          drawer: _initialLoadComplete
              ? FederationSidebar(
                  key: ValueKey(_refreshTrigger),
                  federationsFuture: _federationFuture,
                  onFederationSelected: _setSelectedFederation,
                )
              : null,
          body: bodyContent,
          bottomNavigationBar: _initialLoadComplete
              ? BottomNavigationBar(
                  currentIndex: _currentIndex,
                  onTap: (index) => _onNavBarTapped(index, innerContext),
                  selectedItemColor: _currentIndex == 0 ? Colors.grey : Colors.greenAccent,
                  unselectedItemColor: Colors.grey,
                  items: const [
                    BottomNavigationBarItem(
                      icon: Icon(Icons.qr_code_scanner),
                      label: 'Scan',
                    ),
                    BottomNavigationBarItem(
                      icon: Icon(Icons.settings),
                      label: "Settings",
                    )
                  ],
                )
              : null,
        ),
      ),
    );
  }
}


