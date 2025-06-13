import 'package:flutter/material.dart';
import 'package:carbine/multimint.dart';
import 'package:carbine/utils.dart';

class PendingDepositItem extends StatelessWidget {
  final DepositEventKind event;

  const PendingDepositItem({super.key, required this.event});

  @override
  Widget build(BuildContext context) {
    String msg;
    BigInt amount;

    switch (event) {
      case DepositEventKind_Mempool(field0: final e):
        msg = 'Tx in mempool';
        amount = e.amount;
        break;
      case DepositEventKind_AwaitingConfs(field0: final e):
        msg = 'Tx in block ${e.blockHeight}. Remaining confs: ${e.needed}';
        amount = e.amount;
        break;
      case DepositEventKind_Confirmed(field0: final e):
        msg = 'Tx confirmed, claiming ecash';
        amount = e.amount;
        break;
      case DepositEventKind_Claimed():
        return const SizedBox.shrink();
    }

    final formatted = formatBalance(amount, false);
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
          child: const Icon(Icons.link, color: Colors.yellowAccent),
        ),
        title: Text(
          "Pending Receive",
          style: Theme.of(context).textTheme.bodyMedium,
        ),
        subtitle: Text(msg, style: Theme.of(context).textTheme.bodyMedium),
        trailing: Text(formatted, style: amountStyle),
      ),
    );
  }
}
