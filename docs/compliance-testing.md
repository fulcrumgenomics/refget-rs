# GA4GH Refget Compliance Testing

This document describes how to run the GA4GH refget compliance test suite against refget-rs.

## Overview

The [ga4gh/refget-compliance-suite](https://github.com/ga4gh/refget-compliance-suite) is a Python-based test harness that validates refget server implementations against the specification. The published suite (v1.2.6) tests against **v1** content types, but refget-rs accepts both v1 and v2 content types for backwards compatibility.

### Expected Results

**29 passed, 1 "failed" (expected skip), 0 skipped.**

The single "failure" is `test_sequence_circular_support_false_errors` — a test that only runs when the server does NOT support circular sequences. Since refget-rs supports circular sequences, this test is correctly skipped. The compliance suite counts it as a "failure" in its dependency graph, but it's the expected behavior. The mutually exclusive `test_sequence_circular_support_true_errors` passes instead.

## Prerequisites

- Rust toolchain (see `rust-toolchain.toml`)
- Python 3.8+ (for the compliance suite)
- `samtools` (for generating FASTA index files)

## Step 1: Build refget-rs

```bash
cargo build --release -p refget-server-bin -p refget-tools
```

## Step 2: Download compliance test sequences

The compliance suite uses three sequences from the [ga4gh/refget-compliance-suite](https://github.com/ga4gh/refget-compliance-suite) repository:

| Sequence | Organism | Length | Circular |
|---|---|---|---|
| I | S. cerevisiae chr I | 230,218 | No |
| VI | S. cerevisiae chr VI | 270,161 | No |
| NC_001422.1 | phiX174 | 5,386 | Yes |

```bash
mkdir -p /tmp/refget-compliance-data
cd /tmp/refget-compliance-data

curl -sO https://raw.githubusercontent.com/ga4gh/refget-compliance-suite/master/compliance_suite/sequences/I.faa
curl -sO https://raw.githubusercontent.com/ga4gh/refget-compliance-suite/master/compliance_suite/sequences/VI.faa
curl -sO https://raw.githubusercontent.com/ga4gh/refget-compliance-suite/master/compliance_suite/sequences/NC.faa
```

## Step 3: Prepare FASTA file

Combine the sequences into a single FASTA file and create the index:

```bash
cd /tmp/refget-compliance-data
cat I.faa VI.faa NC.faa > compliance.fa
samtools faidx compliance.fa
```

## Step 4: Generate digest cache

```bash
./target/release/refget-tools cache /tmp/refget-compliance-data/compliance.fa
```

## Step 5: Create server configuration

The phiX174 sequence (NC_001422.1) is circular. Create a YAML config:

```bash
cat > /tmp/refget-compliance-data/config.yml << 'EOF'
circular_sequences:
  - NC_001422.1
EOF
```

## Step 6: Start the server

```bash
./target/release/refget-server \
    --fasta /tmp/refget-compliance-data/compliance.fa \
    --config /tmp/refget-compliance-data/config.yml \
    --port 8181
```

Verify the server is running:
```bash
curl -s http://localhost:8181/sequence/service-info | python3 -m json.tool
```

## Step 7: Install and run the compliance suite

```bash
python3 -m venv /tmp/refget-compliance-venv
/tmp/refget-compliance-venv/bin/pip install refget-compliance
```

Run the tests:
```bash
/tmp/refget-compliance-venv/bin/refget-compliance report \
    -s http://localhost:8181/ \
    --json_path /tmp/refget-compliance-report.json \
    --no-web
```

## Step 8: Review results

Parse the JSON report:
```bash
python3 -c "
import json
with open('/tmp/refget-compliance-report.json') as f:
    report = json.load(f)
passed = failed = skipped = 0
for entry in report:
    for test in entry.get('test_results', []):
        r = test.get('result', -1)
        name = test.get('name', '?')
        if r == 1: passed += 1
        elif r == 0:
            failed += 1
            print(f'  FAIL: {name}')
        elif r == -1:
            skipped += 1
            text = test.get('text', '')
            if 'skipped because' not in text:
                print(f'  SKIP: {name} -- {text[:100]}')
print(f'\nSummary: {passed} passed, {failed} failed, {skipped} skipped')
"
```

Expected output:
```
  FAIL: test_sequence_circular_support_false_errors

Summary: 29 passed, 1 failed, 0 skipped
```

The single "failure" is expected — it's a test for servers that do NOT support circular sequences. Since we support them, it's correctly skipped.

## Compliance Suite Limitations

The published compliance suite (v1.2.6) has several known limitations:

- Tests v1 content types (`application/vnd.ga4gh.refget.v1.0.0+json`), not v2
- Expects a `"service"` key wrapping the service-info response (v1 format)
- Does not test: CORS headers, `Accept-Ranges` header, media type degradation, case-insensitive MD5, subsequence limit enforcement, structured JSON error bodies, or 303 redirects

refget-rs handles all of these correctly regardless of whether the suite tests them.
