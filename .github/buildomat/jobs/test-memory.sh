#!/bin/bash
#:
#: name = "test-memory"
#: variety = "basic"
#: target = "helios"
#: output_rules = [
#:  "/tmp/test_mem_log.txt",
#:  "/tmp/dsc/*.txt",
#: ]
#: skip_clone = true
#:
#: [dependencies.rbuild]
#: job = "rbuild"

input="/input/rbuild/work"

set -o errexit
set -o pipefail
set -o xtrace

banner unpack
mkdir -p /var/tmp/bins
for t in "$input/rbins/"*.gz; do
	b=$(basename "$t")
	b=${b%.gz}
	gunzip < "$t" > "/var/tmp/bins/$b"
	chmod +x "/var/tmp/bins/$b"
done

export BINDIR=/var/tmp/bins

banner setup
pfexec plimit -n 9123456 $$

banner start
ptime -m bash $input/scripts/test_mem.sh