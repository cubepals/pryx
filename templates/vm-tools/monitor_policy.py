#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Translate the pinned monitor's restrictive policy to its equivalent native libc ABI.

Thread signals, pthread joins, event waits, terminal attributes and file metadata (statx, where
the published policy allows stat) are translated. Keep every other syscall, argument constraint
and trap default. GNU's empty upstream
policy is never an input. The launcher's independent self-TGID restriction remains required.
This produces build data, never a workload-selected monitor option.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path

# Linux tkill(2), tgkill(2) and futex(2) document these argument positions; no
# Python library exports them. All signal/futex values come from the actual ABI helper.
TKILL_SIGNAL_ARGUMENT = 1
TGKILL_SIGNAL_ARGUMENT = 2
FUTEX_OPERATION_ARGUMENT = 1
FUTEX_BITSET_ARGUMENT = 5


def adapt(original, abi):
    if not original or any(profile['default_action'] != 'trap' or profile['filter_action'] != 'allow'
                           for profile in original.values()):
        raise ValueError('A restrictive published monitor policy is required')
    signals = {argument['val'] for profile in original.values() for rule in profile['filter']
               if rule['syscall'] == 'tkill' for argument in rule.get('args', [])
               if argument['index'] == TKILL_SIGNAL_ARGUMENT}
    kicks = signals - {abi['abort']}
    if abi['abort'] not in signals or len(kicks) != 1:
        raise ValueError('Published thread signal policy differs from the reviewed ABI')
    mapping = {abi['abort']: abi['abort'], kicks.pop(): abi['vcpu']}
    result = copy.deepcopy(original)
    changed = {'signals': 0, 'signalIdentity': 0, 'joins': 0, 'eventWaits': 0, 'terminals': 0, 'metadata': 0}
    for profile in result.values():
        signals_process = False
        reads_metadata = False
        for rule in profile['filter']:
            if rule['syscall'] == 'tkill':
                arguments = rule.get('args', [])
                if (len(arguments) != 1 or arguments[0]['index'] != TKILL_SIGNAL_ARGUMENT
                        or arguments[0]['type'] != 'dword' or arguments[0]['op'] != 'eq'):
                    raise ValueError('Unreviewed thread signal argument shape')
                arguments[0]['val'] = mapping[arguments[0]['val']]
                arguments[0]['index'] = TGKILL_SIGNAL_ARGUMENT
                rule['syscall'] = 'tgkill'
                rule['comment'] = 'Native thread signalling; the launcher independently requires its own TGID'
                changed['signals'] += 1
                signals_process = True
            elif rule['syscall'] == 'ioctl':
                terminals = {abi['publishedTerminalGet']: abi['terminalGet'],
                             abi['publishedTerminalSet']: abi['terminalSet']}
                arguments = rule.get('args', [])
                if len(arguments) == 1 and arguments[0].get('val') in terminals:
                    argument = arguments[0]
                    if argument['index'] != 1 or argument['type'] != 'dword' or argument['op'] != 'eq':
                        raise ValueError('Unreviewed terminal request constraint')
                    argument['val'] = terminals[argument['val']]
                    argument['comment'] = 'Equivalent native libc terminal attributes request'
                    changed['terminals'] += 1
            elif rule['syscall'] == 'stat':
                if rule.get('args'):
                    raise ValueError('Unreviewed published file metadata constraints')
                reads_metadata = True
            elif rule['syscall'] == 'epoll_pwait':
                if rule.get('args'):
                    raise ValueError('Unreviewed published event-wait constraints')
                rule['syscall'] = 'epoll_wait'
                rule['comment'] = 'Native libc event wait; no temporary signal-mask argument'
                changed['eventWaits'] += 1
            elif rule['syscall'] == 'futex':
                operations = [argument for argument in rule.get('args', [])
                              if argument['index'] == FUTEX_OPERATION_ARGUMENT
                              and argument['op'] == 'eq' and argument['type'] == 'dword'
                              and argument['val'] == abi['futexWait']]
                if not operations:
                    continue
                if len(operations) != 1 or any(argument['index'] == FUTEX_BITSET_ARGUMENT
                                               for argument in rule['args']):
                    raise ValueError('Unreviewed pthread join constraint shape')
                operations[0]['val'] = abi['futexJoin']
                operations[0]['comment'] = 'Native pthread join wait with its clock selection'
                rule['args'].append({'index': FUTEX_BITSET_ARGUMENT, 'type': 'dword', 'op': 'eq',
                                     'val': abi['futexBitsetAny'], 'comment': 'Full wait bitset only'})
                changed['joins'] += 1
        if reads_metadata:
            # The published rule serves std::fs::File::metadata in snapshot creation. On GNU, Rust's
            # std reads metadata with statx, falling back to fstat only when statx is missing.
            if any(rule['syscall'] == 'statx' for rule in profile['filter']):
                raise ValueError('Published file metadata policy needs renewed review')
            profile['filter'].append({'syscall': 'statx',
                'comment': 'GNU std::fs::File::metadata, as the published stat rule is for musl'})
            changed['metadata'] += 1
        if signals_process:
            if any(rule['syscall'] == 'getpid' for rule in profile['filter']):
                raise ValueError('Published signal identity policy needs renewed review')
            profile['filter'].append({'syscall': 'getpid',
                'comment': 'GNU thread signalling reads only its own process identity before tgkill'})
            changed['signalIdentity'] += 1
    if changed['terminals'] != 2:
        raise ValueError('Expected precisely the two published terminal attribute constraints')
    if not all(changed.values()):
        raise ValueError('The expected native thread ABI substitutions were not found')
    return result, changed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['policy', 'abi', 'output', 'receipt']:
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    original = args.policy.read_bytes()
    generated, changed = adapt(json.loads(original), json.loads(args.abi.read_text()))
    output = (json.dumps(generated, indent=2) + '\n').encode()
    args.output.write_bytes(output)
    args.receipt.write_text(json.dumps({
        'sourcePolicySha256': hashlib.sha256(original).hexdigest(),
        'nativePolicySha256': hashlib.sha256(output).hexdigest(),
        'adaptedRules': changed,
        'selfIdentitySyscall': json.loads(args.abi.read_text())['processId'],
        'nativeEventWaitSyscall': json.loads(args.abi.read_text())['epollWait'],
        'publishedEventWaitSyscall': json.loads(args.abi.read_text())['epollPwait'],
        'ruleCounts': {name: len(profile['filter']) for name, profile in generated.items()},
        'otherSyscallsAndConstraintsChanged': False,
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
