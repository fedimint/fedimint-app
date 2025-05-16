import 'dart:io';

import 'package:carbine/app.dart';
import 'package:carbine/lib.dart';
import 'package:flutter/material.dart';
import 'create_wallet.dart';

class Splash extends StatefulWidget {
  final Directory dir;
  const Splash({super.key, required this.dir});

  @override
  State<Splash> createState() => _SplashState();
}

class _SplashState extends State<Splash> {
  @override
  void initState() {
    super.initState();
    _checkWalletStatus();
  }

  Future<void> _checkWalletStatus() async {
    final exists = await walletExists(path: widget.dir.path);

    if (!mounted) return;
    final Widget screen;
    if (exists) {
      await loadMultimint(path: widget.dir.path);
      final initialFeds = await federations();
      screen = MyApp(initialFederations: initialFeds);
    } else {
      screen = CreateWallet(dir: widget.dir);
    }

    Future.delayed(const Duration(seconds: 2), () async {
      Navigator.of(
        context,
      ).pushReplacement(MaterialPageRoute(builder: (_) => screen));
    });
  }

  @override
  Widget build(BuildContext context) {
    return const Scaffold(
      body: Center(
        child: Image(
          image: AssetImage('assets/images/fedimint.png'),
          width: 200,
        ),
      ),
    );
  }
}
