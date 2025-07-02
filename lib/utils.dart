import 'dart:io';

import 'package:carbine/lib.dart';
import 'package:carbine/models.dart';
import 'package:carbine/multimint.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:path_provider/path_provider.dart';
import 'package:url_launcher/url_launcher.dart';

extension MilliSats on BigInt {
  BigInt get toSats => this ~/ BigInt.from(1000);
}

class AppLogger {
  static late final File _logFile;
  static final AppLogger instance = AppLogger._internal();

  AppLogger._internal();

  static Future<void> init() async {
    Directory? dir;
    if (Platform.isAndroid) {
      dir = await getExternalStorageDirectory();
    } else {
      dir = await getApplicationDocumentsDirectory();
    }
    _logFile = File('${dir!.path}/carbine/carbine.txt');

    if (!(await _logFile.exists())) {
      await _logFile.create(recursive: true);
    }

    instance.info("Logger initialized. Log file: ${_logFile.path}");
  }

  void _log(String level, String message) {
    final timestamp = DateTime.now().toIso8601String();
    final formatted = "[$timestamp] [$level] $message";

    // Print to console
    debugPrint(formatted);

    // Write to file
    _logFile.writeAsStringSync(
      "$formatted\n",
      mode: FileMode.append,
      flush: true,
    );
  }

  void _rustLog(LogLevel level, String message) {
    String logLevel;
    switch (level) {
      case LogLevel.trace:
        logLevel = "TRACE";
        break;
      case LogLevel.error:
        logLevel = "ERROR";
        break;
      case LogLevel.info:
        logLevel = "INFO";
        break;
      case LogLevel.debug:
        logLevel = "DEBUG";
        break;
      case LogLevel.warn:
        logLevel = "WARN";
        break;
    }

    final timestamp = DateTime.now().toIso8601String();
    final formatted = "[$timestamp] [RUST] [$logLevel] $message";

    // Print to console
    debugPrint(formatted);

    // Write to file
    _logFile.writeAsStringSync(
      "$formatted\n",
      mode: FileMode.append,
      flush: true,
    );
  }

  void info(String message) => _log("INFO", message);
  void warn(String message) => _log("WARN", message);
  void error(String message) => _log("ERROR", message);
  void debug(String message) => _log("DEBUG", message);
  void rustLog(LogLevel level, String message) => _rustLog(level, message);
}

int threshold(int totalPeers) {
  final maxEvil = (totalPeers - 1) ~/ 3;
  return totalPeers - maxEvil;
}

String formatBalance(BigInt? msats, bool showMsats) {
  if (msats == null) return showMsats ? '₿0.000' : '₿0';

  if (showMsats) {
    final btcAmount =
        msats.toDouble() / 1000; // convert to sats with msat precision
    final formatter = NumberFormat('#,##0.000', 'en_US');
    var formatted = formatter.format(btcAmount).replaceAll(',', ' ');
    return '₿$formatted';
  } else {
    final sats = msats.toSats;
    final formatter = NumberFormat('#,##0', 'en_US');
    var formatted = formatter.format(sats.toInt()).replaceAll(',', ' ');
    return '₿$formatted';
  }
}

// TODO: Change name
String getAbbreviatedInvoice(String invoice) {
  if (invoice.length <= 14) return invoice;
  return '${invoice.substring(0, 7)}...${invoice.substring(invoice.length - 7)}';
}

String calculateUsdValue(double? btcPrice, int sats) {
  if (btcPrice == null) return '';
  final usdValue = (btcPrice * sats) / 100000000;
  return '\$${usdValue.toStringAsFixed(2)}';
}

int getModuleIdForPaymentType(PaymentType paymentType) {
  switch (paymentType) {
    case PaymentType.lightning:
      return 0;
    case PaymentType.ecash:
      return 1;
    case PaymentType.onchain:
      return 2;
  }
}

Future<double?> fetchBtcPrice() async {
  try {
    final price = await getBtcPrice();
    if (price != null) {
      return price.toDouble();
    }
  } catch (e) {
    AppLogger.instance.error("Error fetching price: $e");
  }

  return null;
}

Future<void> showExplorerConfirmation(BuildContext context, Uri url) async {
  final confirmed = await showDialog<bool>(
    context: context,
    builder:
        (context) => AlertDialog(
          title: const Text('External Link Warning'),
          content: const Text(
            'You are about to navigate to an external block explorer. '
            'Before accepting, please consider the privacy implications '
            'and consider using a self hosted block explorer.',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(false),
              child: const Text('Cancel'),
            ),
            TextButton(
              onPressed: () => Navigator.of(context).pop(true),
              child: const Text('Confirm'),
            ),
          ],
        ),
  );

  if (confirmed == true && await canLaunchUrl(url)) {
    await launchUrl(url, mode: LaunchMode.externalApplication);
  }
}
