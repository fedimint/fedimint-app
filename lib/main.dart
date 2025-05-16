import 'dart:io';

import 'package:carbine/frb_generated.dart';
import 'package:carbine/splash.dart';
import 'package:carbine/theme.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:path_provider/path_provider.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final dir = await getApplicationDocumentsDirectory();
  runApp(Carbine(dir: dir));
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

class Carbine extends StatelessWidget {
  final Directory dir;
  const Carbine({super.key, required this.dir});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: "Carbine",
      debugShowCheckedModeBanner: false,
      theme: cypherpunkNinjaTheme,
      home: Splash(dir: dir),
    );
  }
}
