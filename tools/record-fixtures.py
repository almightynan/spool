#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Record protocol fixtures from real devices for the protocols crate tests.

Only read-only requests are sent: Get-Printer-Attributes, Get-Jobs,
CUPS-Get-Printers, CUPS-Get-Default, eSCL GETs and one mDNS query.

Usage: tools/record-fixtures.py --printer 192.168.1.147 --queue HP_ENVY_5530 OUT_DIR
"""
import argparse
import os
import socket
import struct
import sys
import time
import urllib.request

# RFC 8010 section 3 value tags used in requests.
TAG_OPERATION = 0x01
TAG_END = 0x03
TAG_KEYWORD = 0x44
TAG_URI = 0x45
TAG_CHARSET = 0x47
TAG_LANGUAGE = 0x48
TAG_NAME = 0x42


def attr(tag, name, value):
    v = value.encode()
    return struct.pack(">BH", tag, len(name)) + name.encode() + struct.pack(">H", len(v)) + v


def additional(tag, value):
    v = value.encode()
    return struct.pack(">BH", tag, 0) + struct.pack(">H", len(v)) + v


def ipp_request(op, uri, extra=b""):
    body = struct.pack(">BBHI", 2, 0, op, 1)
    body += bytes([TAG_OPERATION])
    body += attr(TAG_CHARSET, "attributes-charset", "utf-8")
    body += attr(TAG_LANGUAGE, "attributes-natural-language", "en")
    if uri:
        body += attr(TAG_URI, "printer-uri", uri)
    body += attr(TAG_NAME, "requesting-user-name", "fixture")
    body += extra
    body += bytes([TAG_END])
    return body


def post_ipp(host, port, path, body):
    s = socket.create_connection((host, port), timeout=10)
    req = (
        f"POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/ipp\r\n"
        f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
    ).encode() + body
    s.sendall(req)
    f = s.makefile("rb")
    status = f.readline().decode().strip()
    headers = {}
    while True:
        line = f.readline()
        if line in (b"\r\n", b"\n", b""):
            break
        k, _, v = line.decode().partition(":")
        headers[k.strip().lower()] = v.strip()
    if headers.get("transfer-encoding", "").lower() == "chunked":
        out = b""
        while True:
            n = int(f.readline().split(b";")[0], 16)
            if n == 0:
                f.readline()
                break
            out += f.read(n)
            f.readline()
    else:
        out = f.read(int(headers.get("content-length", "0")))
    s.close()
    return status, out


def requested(*names):
    out = attr(TAG_KEYWORD, "requested-attributes", names[0])
    for n in names[1:]:
        out += additional(TAG_KEYWORD, n)
    return out


def mdns_query(service):
    """Send one multicast PTR query and collect raw answers for 3 seconds."""
    q = struct.pack(">HHHHHH", 0, 0, 1, 0, 0, 0)
    for label in service.split("."):
        q += bytes([len(label)]) + label.encode()
    q += b"\x00" + struct.pack(">HH", 12, 1)
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.settimeout(0.5)
    s.sendto(q, ("224.0.0.251", 5353))
    packets = []
    end = time.time() + 3
    while time.time() < end:
        try:
            data, _ = s.recvfrom(9000)
            packets.append(data)
        except socket.timeout:
            pass
    return packets


def parse_name(buf, off):
    labels = []
    jumped = False
    ret = off
    while True:
        n = buf[off]
        if n == 0:
            off += 1
            break
        if n & 0xC0 == 0xC0:
            ptr = ((n & 0x3F) << 8) | buf[off + 1]
            if not jumped:
                ret = off + 2
            jumped = True
            off = ptr
            continue
        labels.append(buf[off + 1 : off + 1 + n].decode("utf-8", "replace"))
        off += 1 + n
    return ".".join(labels), (ret if jumped else off)


def txt_records(packets, service):
    """Return {instance: [txt strings]} for TXT records under the service."""
    found = {}
    for p in packets:
        try:
            _, _, qd, an, ns, ar = struct.unpack(">HHHHHH", p[:12])
            off = 12
            for _ in range(qd):
                _, off = parse_name(p, off)
                off += 4
            for _ in range(an + ns + ar):
                name, off = parse_name(p, off)
                rtype, _, _, rdlen = struct.unpack(">HHIH", p[off : off + 10])
                off += 10
                rdata = p[off : off + rdlen]
                off += rdlen
                if rtype == 16 and name.endswith(service):
                    strings = []
                    i = 0
                    while i < len(rdata):
                        n = rdata[i]
                        strings.append(rdata[i + 1 : i + 1 + n].decode("utf-8", "replace"))
                        i += 1 + n
                    found[name] = strings
        except (struct.error, IndexError):
            continue
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--printer", required=True)
    ap.add_argument("--queue", required=True)
    ap.add_argument("out")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)

    def save(name, data):
        with open(os.path.join(a.out, name), "wb") as f:
            f.write(data)
        print(f"wrote {name} ({len(data)} bytes)")

    # Printer, directly (PWG 5100.14 resource /ipp/print).
    uri = f"ipp://{a.printer}:631/ipp/print"
    st, body = post_ipp(a.printer, 631, "/ipp/print", ipp_request(0x000B, uri, requested("all")))
    print(st)
    save("printer-get-printer-attributes.ipp", body)

    # Local CUPS.
    quri = f"ipp://localhost:631/printers/{a.queue}"
    st, body = post_ipp("localhost", 631, f"/printers/{a.queue}", ipp_request(0x000B, quri, requested("all")))
    print(st)
    save("cups-queue-get-printer-attributes.ipp", body)
    st, body = post_ipp("localhost", 631, "/", ipp_request(0x4002, None, requested(
        "printer-name", "printer-uri-supported", "device-uri", "printer-info", "printer-location",
        "printer-make-and-model", "printer-state", "printer-state-reasons", "printer-is-accepting-jobs",
        "printer-type", "printer-uuid", "printer-more-info")))
    print(st)
    save("cups-get-printers.ipp", body)
    for which in ("completed", "not-completed"):
        st, body = post_ipp("localhost", 631, f"/printers/{a.queue}", ipp_request(
            0x000A, quri, attr(TAG_KEYWORD, "which-jobs", which) + requested(
                "job-id", "job-name", "job-state", "job-state-reasons", "job-originating-user-name",
                "job-k-octets", "time-at-creation", "time-at-completed", "job-printer-uri",
                "job-impressions-completed")))
        print(st)
        save(f"cups-get-jobs-{which}.ipp", body)
    st, body = post_ipp("localhost", 631, "/printers/does-not-exist", ipp_request(
        0x000B, "ipp://localhost:631/printers/does-not-exist", requested("printer-state")))
    print(st)
    save("cups-not-found.ipp", body)
    st, body = post_ipp("localhost", 631, "/", ipp_request(0x4001, None, requested("printer-name")))
    print(st)
    save("cups-get-default.ipp", body)

    # eSCL.
    for res in ("ScannerCapabilities", "ScannerStatus"):
        with urllib.request.urlopen(f"http://{a.printer}/eSCL/{res}", timeout=10) as r:
            save(f"escl-{res}.xml", r.read())

    # DNS-SD TXT records.
    for svc in ("_ipp._tcp.local", "_ipps._tcp.local", "_uscan._tcp.local", "_uscans._tcp.local"):
        recs = txt_records(mdns_query(svc), svc)
        text = ""
        for inst, strings in sorted(recs.items()):
            text += f"# {inst}\n" + "\n".join(strings) + "\n"
        save(f"dnssd-{svc.split('.')[0].lstrip('_')}-txt.txt", text.encode())


if __name__ == "__main__":
    sys.exit(main())
