"""Strict observation checks for control-flow; this does not execute the scenario.

The caller must supply public events and every public output stream. In particular,
protected agent output is not a substitute for the manifest's public loop stdout.
"""
import pwd

from expectations_runner import path_in, require


def task_result(events, host, task):
    matches = [event['result'] for event in events
               if event.get('host') == host and event.get('task') == task and 'result' in event]
    require(len(matches) == 1 and isinstance(matches[0], dict), f'{task}: missing or duplicate result')
    return matches[0]


def assert_observations(root, observations, events, outputs):
    require(isinstance(observations, list) and observations, 'missing control-flow observations')
    require(isinstance(outputs, dict) and set(outputs) == {'stdout', 'stderr', 'log'}
            and all(isinstance(value, str) for value in outputs.values()), 'missing public output stream')
    for item in observations:
        require(isinstance(item, dict), 'invalid observation')
        kind = item.get('kind')
        if kind == 'file':
            require(set(item) == {'kind', 'path', 'content', 'owner'}
                    and isinstance(item['content'], str) and isinstance(item['owner'], str), 'invalid file observation')
            target = path_in(root, item['path'])
            require(target.is_file() and not target.is_symlink(), 'missing or unsafe observed file')
            require(target.read_bytes() == item['content'].encode(), 'file content mismatch')
            require(target.stat().st_uid == pwd.getpwnam(item['owner']).pw_uid, 'file owner mismatch')
        elif kind == 'file_exists':
            require(set(item) == {'kind', 'path'}, 'invalid file existence observation')
            target = path_in(root, item['path'])
            require(target.is_file() and not target.is_symlink(), 'missing or unsafe observed file')
        elif kind == 'task_result':
            require(set(item) == {'kind', 'host', 'task', 'field', 'equals'}, 'invalid task result observation')
            result = task_result(events, item['host'], item['task'])
            require(item['field'] in result and type(result[item['field']]) is type(item['equals'])
                    and result[item['field']] == item['equals'], 'task result mismatch')
        elif kind == 'task_loop_stdout':
            require(set(item) == {'kind', 'host', 'task', 'values'} and isinstance(item['values'], list)
                    and all(isinstance(value, str) for value in item['values']), 'invalid loop observation')
            result = task_result(events, item['host'], item['task'])
            values = result.get('results')
            require(isinstance(values, list) and len(values) == len(item['values'])
                    and all(isinstance(value, dict) and isinstance(value.get('stdout'), str) for value in values),
                    'loop stdout missing, censored, or wrong cardinality')
            require([value['stdout'] for value in values] == item['values'], 'loop stdout mismatch')
        elif kind == 'fact':
            require(set(item) == {'kind', 'host', 'name', 'equals'}, 'invalid fact observation')
            result = task_result(events, item['host'], 'Record rescued synthetic state')
            facts = result.get('facts')
            require(isinstance(facts, dict) and item['name'] in facts
                    and type(facts[item['name']]) is type(item['equals']) and facts[item['name']] == item['equals'], 'rescue fact mismatch')
        elif kind == 'output_excludes':
            require(set(item) == {'kind', 'values'} and isinstance(item['values'], list) and item['values']
                    and all(isinstance(value, str) and value for value in item['values']), 'invalid output exclusion')
            require(all(value not in stream for value in item['values'] for stream in outputs.values()),
                    'excluded value appears in public output')
        else:
            raise RuntimeError(f'unsupported control-flow observation: {kind}')


def assert_retry_attempts(expected, observed):
    require(type(expected) is int and expected > 0 and type(observed) is int and observed == expected,
            'retry attempt count missing or mismatched')
