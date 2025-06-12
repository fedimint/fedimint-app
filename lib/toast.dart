import 'dart:async';
import 'package:flutter/material.dart';

class ToastService {
  static final ToastService _instance = ToastService._internal();
  factory ToastService() => _instance;
  ToastService._internal();

  final GlobalKey<NavigatorState> navigatorKey = GlobalKey<NavigatorState>();
  OverlayEntry? _currentToast;

  void show({
    required String message,
    required Duration duration,
    required VoidCallback onTap,
  }) {
    final context = navigatorKey.currentContext;
    if (context == null) {
      debugPrint('ToastService: No context available');
      return;
    }

    _currentToast?.remove();

    final progressNotifier = ValueNotifier<double>(1.0);
    final overlay = Overlay.of(context);

    final entry = OverlayEntry(
      builder: (context) {
        final theme = Theme.of(context);
        return Positioned(
          bottom: 50,
          left: 20,
          right: 20,
          child: GestureDetector(
            onTap: () {
              onTap();
              _currentToast?.remove();
              _currentToast = null;
            },
            child: Material(
              color: Colors.transparent,
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 12,
                ),
                decoration: BoxDecoration(
                  color: theme.colorScheme.surface.withOpacity(0.95),
                  borderRadius: BorderRadius.circular(16),
                  boxShadow: [
                    BoxShadow(
                      color: Colors.greenAccent.withOpacity(0.5),
                      blurRadius: 10,
                      spreadRadius: 1,
                    ),
                  ],
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      message,
                      style: theme.textTheme.bodyLarge?.copyWith(
                        color: Colors.white,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                    const SizedBox(height: 8),
                    ValueListenableBuilder<double>(
                      valueListenable: progressNotifier,
                      builder: (context, value, _) {
                        return ClipRRect(
                          borderRadius: BorderRadius.circular(4),
                          child: LinearProgressIndicator(
                            value: value,
                            minHeight: 4,
                            backgroundColor: Colors.white10,
                            valueColor: AlwaysStoppedAnimation(
                              theme.colorScheme.primary,
                            ),
                          ),
                        );
                      },
                    ),
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );

    _currentToast = entry;
    overlay.insert(entry);

    final startTime = DateTime.now();
    Timer.periodic(const Duration(milliseconds: 50), (timer) {
      final elapsed = DateTime.now().difference(startTime).inMilliseconds;
      final total = duration.inMilliseconds;
      if (elapsed >= total) {
        timer.cancel();
        _currentToast?.remove();
        _currentToast = null;
      } else {
        progressNotifier.value = 1 - elapsed / total;
      }
    });
  }
}
