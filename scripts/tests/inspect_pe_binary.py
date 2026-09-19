#!/usr/bin/env python3
"""
PE32+ (x86_64 and ARM64) Binary Inspector
Reads PE headers, machine types, subsystems, and imported DLL names directly.
Zero third-party dependencies.
"""

import sys
import struct
import os

MACHINE_TYPES = {
    0x8664: "AMD64 (x86_64)",
    0xAA64: "ARM64 (AArch64 Windows 11)",
    0x014C: "Intel 386 (i386)",
    0x01C0: "ARM (Thumb-2)",
}

SUBSYSTEMS = {
    1: "IMAGE_SUBSYSTEM_NATIVE",
    2: "IMAGE_SUBSYSTEM_WINDOWS_GUI",
    3: "IMAGE_SUBSYSTEM_WINDOWS_CUI (Console)",
    7: "IMAGE_SUBSYSTEM_POSIX_CUI",
    9: "IMAGE_SUBSYSTEM_WINDOWS_CE_GUI",
    10: "IMAGE_SUBSYSTEM_EFI_APPLICATION",
}

def inspect_pe(file_path):
    print(f"\n===============================================================================")
    print(f"Inspecting PE Binary: {os.path.basename(file_path)}")
    print(f"Path: {file_path}")
    print(f"File Size: {os.path.getsize(file_path):,} bytes")
    print(f"===============================================================================")

    with open(file_path, "rb") as f:
        data = f.read()

    if len(data) < 64 or data[:2] != b"MZ":
        print("❌ Not a valid DOS/PE executable (missing MZ signature)")
        return False

    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if pe_offset + 24 > len(data) or data[pe_offset:pe_offset+4] != b"PE\x00\x00":
        print("❌ Missing PE signature at offset 0x3C")
        return False

    # COFF File Header (20 bytes)
    coff_header = data[pe_offset+4:pe_offset+24]
    machine, num_sections, timedate, sym_ptr, num_syms, opt_hdr_size, chars = struct.unpack(
        "<HHIIIHH", coff_header
    )

    machine_str = MACHINE_TYPES.get(machine, f"Unknown (0x{machine:04X})")
    print(f"PE Machine Type:      0x{machine:04X} -> {machine_str}")
    print(f"Number of Sections:   {num_sections}")
    print(f"Characteristics:      0x{chars:04X}")

    # Optional Header (PE32+ / 64-bit)
    opt_offset = pe_offset + 24
    magic = struct.unpack_from("<H", data, opt_offset)[0]
    is_pe32_plus = (magic == 0x020B)
    print(f"Optional Header Magic:0x{magic:04X} ({'PE32+ 64-bit' if is_pe32_plus else 'PE32 32-bit'})")

    if not is_pe32_plus:
        print("❌ Expected PE32+ (64-bit)")
        return False

    subsystem = struct.unpack_from("<H", data, opt_offset + 68)[0]
    subsystem_str = SUBSYSTEMS.get(subsystem, f"Unknown ({subsystem})")
    print(f"Subsystem:            {subsystem} -> {subsystem_str}")

    # Section Headers
    sections = []
    sec_offset = opt_offset + opt_hdr_size
    for s_idx in range(num_sections):
        sec_hdr = data[sec_offset : sec_offset + 40]
        name = sec_hdr[:8].rstrip(b"\x00").decode("ascii", errors="replace")
        v_size, v_addr, raw_size, raw_offset = struct.unpack_from("<IIII", sec_hdr, 8)
        sections.append({
            "name": name,
            "v_addr": v_addr,
            "v_size": v_size,
            "raw_offset": raw_offset,
            "raw_size": raw_size
        })
        sec_offset += 40

    print(f"\nSections ({len(sections)}):")
    for s in sections:
        print(f"  - {s['name']:<10} Virtual: 0x{s['v_addr']:08X} (size {s['v_size']:,}) | Raw: 0x{s['raw_offset']:08X} (size {s['raw_size']:,})")

    # Data Directories
    num_rva_sizes = struct.unpack_from("<I", data, opt_offset + 108)[0]
    data_dirs_offset = opt_offset + 112

    def rva_to_offset(rva):
        for s in sections:
            if s["v_addr"] <= rva < s["v_addr"] + max(s["v_size"], s["raw_size"]):
                return s["raw_offset"] + (rva - s["v_addr"])
        return None

    # Import Table is Directory Index 1
    if num_rva_sizes > 1:
        import_rva, import_size = struct.unpack_from("<II", data, data_dirs_offset + 8)
        if import_rva != 0 and import_size != 0:
            import_raw = rva_to_offset(import_rva)
            if import_raw is not None:
                imported_dlls = []
                idx = 0
                while True:
                    entry_offset = import_raw + idx * 20
                    if entry_offset + 20 > len(data):
                        break
                    ilt_rva, timedate, forwarder, name_rva, iat_rva = struct.unpack_from(
                        "<IIIII", data, entry_offset
                    )
                    if ilt_rva == 0 and name_rva == 0 and iat_rva == 0:
                        break
                    name_offset = rva_to_offset(name_rva)
                    if name_offset is not None:
                        dll_name_end = data.find(b"\x00", name_offset)
                        if dll_name_end != -1:
                            dll_name = data[name_offset:dll_name_end].decode("ascii", errors="replace")
                            imported_dlls.append(dll_name)
                    idx += 1

                print(f"\nImported DLLs ({len(imported_dlls)}):")
                for dll in imported_dlls:
                    print(f"  [DLL] {dll}")

    print(f"\n✓ PE32+ validation completed successfully.")
    return True

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: inspect_pe_binary.py <path_to_pe_binary>")
        sys.exit(1)
    for p in sys.argv[1:]:
        inspect_pe(p)
