<!--
SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Fixtures

Responses recorded from real devices. Do not edit them by hand: tests compare
against the exact bytes. Re-record with `tools/record-fixtures.py`.

| File | Source | Request |
|---|---|---|
| `printer-get-printer-attributes.ipp` | HP ENVY 5530 series, firmware as of 2026-11-19, `ipp://192.168.1.147:631/ipp/print` | Get-Printer-Attributes, requested-attributes `all`, IPP/2.0 |
| `cups-queue-get-printer-attributes.ipp` | CUPS 2.4.10 (Debian 13), queue `HP_ENVY_5530` (IPP Everywhere) | Get-Printer-Attributes `all` |
| `cups-get-printers.ipp` | same CUPS, two queues: one manual, one created by cups-browsed | CUPS-Get-Printers with the queue attribute list |
| `cups-get-jobs-completed.ipp`, `cups-get-jobs-not-completed.ipp` | same CUPS | Get-Jobs, which-jobs `completed` / `not-completed` |
| `cups-not-found.ipp` | same CUPS | Get-Printer-Attributes on a queue that does not exist |
| `cups-get-default.ipp` | same CUPS, no default set | CUPS-Get-Default |
| `escl-ScannerCapabilities.xml`, `escl-ScannerStatus.xml` | HP ENVY 5530 series, `http://192.168.1.147/eSCL/` | GET |
| `escl-thirdparty-hp-m426fdn-*.xml` | HP LaserJet MFP M426fdn, captured by the OpenPrinting go-mfp project (`internal/testutils/data`, BSD-2-Clause, Copyright (c) 2024 Alexander Pevzner). Used because the ENVY 5530 has no document feeder | GET |
| `dnssd-ipp-txt.txt`, `dnssd-uscan-txt.txt` | HP ENVY 5530 series, `_ipp._tcp` and `_uscan._tcp` as resolved by mdns-sd | `SPOOL_RECORD_TXT=<dir> cargo test -p spool-services browse_real -- --ignored` |

The DNS-SD files hold the first line `Kind<TAB>instance<TAB>host<TAB>port` and then
the TXT record as `key=value` lines.
