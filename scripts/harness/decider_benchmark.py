def summary(rows):
    timed = sorted(row['latency_s'] for row in rows if row.get('model_calls', 0))
    model_rows = [row for row in rows if row.get('model_calls', 0)]
    return {'checks': len(rows), 'passed': sum(row['ok'] for row in rows),
            'accuracy': sum(row['ok'] for row in rows) / max(1, len(rows)),
            'model_checks': len(model_rows), 'model_passed': sum(row['ok'] for row in model_rows),
            'model_accuracy': sum(row['ok'] for row in model_rows)/len(model_rows) if model_rows else None,
            'model_calls': sum(row.get('model_calls', 0) for row in rows),
            'latency_p50_s': statistics.median(timed) if timed else None,
            'latency_p95_s': timed[min(len(timed)-1, math.ceil(len(timed)*.95)-1)] if timed else None}


def model_decision(decider, row):
    if row['entry'] == 'join':
        return decider.decide(row['snapshot'], goal=row['goal'], guest_name='Sarah Guest')
    if row['entry'] == 'host':
        return decider.decide_host(row['snapshot'])
    return decider.decide_ui(row['snapshot'], row['goal'])


def navigation_snapshot(snapshot):
    """Keep every control and its state; omit duplicate prose for navigation."""
    fields = ('index', 'text', 'role', 'value', 'disabled', 'checked')
    return {'elements': [{k: item[k] for k in fields if k in item}
                         for item in snapshot['elements']
                         if MeetingDecider._is_clickable(item) or MeetingDecider._is_text_input(item)]}


