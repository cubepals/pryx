#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Translate the pinned monitor's restrictive policy to its equivalent native libc ABI.

Keep every other syscall, argument constraint and trap default. GNU's empty upstream
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
    changed = {'signals': 0, 'joins': 0, 'eventWaits': 0}
    for profile in result.values():
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
        'nativeEventWaitSyscall': json.loads(args.abi.read_text())['epollWait'],
        'publishedEventWaitSyscall': json.loads(args.abi.read_text())['epollPwait'],
        'ruleCounts': {name: len(profile['filter']) for name, profile in generated.items()},
        'otherSyscallsAndConstraintsChanged': False,
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
