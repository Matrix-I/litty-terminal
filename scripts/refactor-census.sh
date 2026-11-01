#!/usr/bin/env bash
# Token census of src/**/*.rs. A pure move keeps every count identical.
# Usage: scripts/refactor-census.sh > census.txt
set -eo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
files=$(find src -name '*.rs' | sort)
count() { # label, ERE
    printf '%-18s %s\n' "$1" "$(cat $files | grep -oE "$2" | wc -l | tr -d ' ')"
}
count lock '\.lock\(\)'
count unsafe '\bunsafe\b'
count cfg '#\[cfg(_attr)?\('
count thread_spawn 'thread::(spawn|Builder)|\.spawn\('
count blocking_io 'std::fs::|File::(open|create)|\.write_all\(|\.read_to_|Command::new|\.output\(\)|\.wait\(\)'
count alloc 'Vec::new|vec!\[|Vec::with_capacity|Box::new|String::new|String::from|format!\(|\.to_string\(\)|\.to_owned\(\)|\.to_vec\(\)|\.clone\(\)|\.collect\(\)'
count test_attr '#\[test\]'
cat $files | grep -oE '\b(WindowEvent|AppEvent|BenchCommand|ElementState|MouseButton|MouseScrollDelta|Ime|NamedKey|Action)::[A-Za-z0-9_]+' | sort | uniq -c | awk '{printf "arm %-40s %s\n", $2, $1}'
