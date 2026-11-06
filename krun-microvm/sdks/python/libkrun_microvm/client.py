"""Client and binary resolution for libkrun_microvm."""

import os
import shutil
import subprocess
import selectors
import time
from pathlib import Path
from typing import List, Dict, Optional, Any, Union
from .exceptions import MicroVmBinaryNotFoundError, MicroVmExecutionError, MicroVmTimeoutError

def find_microvm_binary() -> str:
    """
    Locate the `microvm` CLI binary.
    Checks:
    1. MICROVM_BIN environment variable
    2. In PATH
    3. In common target build directories (debug/release)
    """
    if "MICROVM_BIN" in os.environ:
        bin_path = os.environ["MICROVM_BIN"]
        if os.path.isfile(bin_path) and os.access(bin_path, os.X_OK):
            return bin_path
        raise MicroVmBinaryNotFoundError(f"MICROVM_BIN was specified as '{bin_path}' but file is not executable")

    # In system PATH
    which_path = shutil.which("microvm")
    if which_path:
        return which_path

    # Look relative to repository root
    current = Path(__file__).resolve()
    for parent in current.parents:
        release_bin = parent / "target" / "release" / "microvm"
        if release_bin.is_file() and os.access(release_bin, os.X_OK):
            return str(release_bin)
        debug_bin = parent / "target" / "debug" / "microvm"
        if debug_bin.is_file() and os.access(debug_bin, os.X_OK):
            return str(debug_bin)

    raise MicroVmBinaryNotFoundError(
        "Could not find 'microvm' CLI binary. Build it with 'cargo build --release' "
        "or set MICROVM_BIN=/path/to/microvm"
    )

def build_run_args(
    image: str,
    cmd: Optional[List[str]] = None,
    cpus: Optional[int] = None,
    memory_mb: Optional[int] = None,
    allow_hosts: Optional[List[str]] = None,
    secrets: Optional[Dict[str, str]] = None,
    max_tokens: Optional[int] = None,
    volumes: Optional[List[str]] = None,
    env: Optional[Dict[str, str]] = None,
    workdir: Optional[str] = None,
    no_sandbox: bool = False,
    extra_args: Optional[List[str]] = None,
) -> List[str]:
    """Construct argument list for 'microvm run'."""
    args = ["run"]

    if cpus is not None:
        args.extend(["-c", str(cpus)])
    if memory_mb is not None:
        args.extend(["-m", str(memory_mb)])
    if no_sandbox:
        args.append("--no-sandbox")

    if allow_hosts:
        for host in allow_hosts:
            args.extend(["--allow-host", host])

    if secrets:
        for k, v in secrets.items():
            args.extend(["--secret", f"{k}={v}"])

    if max_tokens is not None:
        args.extend(["--max-tokens", str(max_tokens)])

    if volumes:
        for vol in volumes:
            args.extend(["-v", vol])

    if env:
        for k, v in env.items():
            args.extend(["-e", f"{k}={v}"])

    if workdir:
        args.extend(["-w", workdir])

    if extra_args:
        args.extend(extra_args)

    args.append(image)

    if cmd:
        args.append("--")
        args.extend(cmd)

    return args

MAX_CAPTURE_BYTES = 16 * 1024 * 1024

def execute_microvm_command(
    args: List[str],
    binary_path: Optional[str] = None,
    timeout: Optional[float] = None,
    capture_output: bool = True,
) -> subprocess.CompletedProcess:
    """Execute the microvm CLI command with arguments."""
    bin_path = binary_path or find_microvm_binary()
    full_cmd = [bin_path] + args

    try:
        if not capture_output:
            return subprocess.run(full_cmd, text=True, timeout=timeout, check=False)

        # Drain both pipes concurrently with a shared limit. communicate()/run()
        # otherwise buffer arbitrary guest output in host memory.
        deadline = None if timeout is None else time.monotonic() + timeout
        with subprocess.Popen(full_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as proc:
            stdout = bytearray()
            stderr = bytearray()
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(proc.stdout, selectors.EVENT_READ, stdout)
                    selector.register(proc.stderr, selectors.EVENT_READ, stderr)
                    while selector.get_map():
                        remaining = None if deadline is None else deadline - time.monotonic()
                        if remaining is not None and remaining <= 0:
                            raise subprocess.TimeoutExpired(full_cmd, timeout)
                        for key, _events in selector.select(remaining):
                            chunk = os.read(key.fd, 65536)
                            if not chunk:
                                selector.unregister(key.fileobj)
                                continue
                            if len(stdout) + len(stderr) + len(chunk) > MAX_CAPTURE_BYTES:
                                raise MicroVmExecutionError("MicroVM output exceeds the 16 MiB capture limit")
                            key.data.extend(chunk)
                remaining = None if deadline is None else max(0, deadline - time.monotonic())
                returncode = proc.wait(timeout=remaining)
            except BaseException:
                proc.kill()
                proc.wait()
                raise
        return subprocess.CompletedProcess(
            full_cmd, returncode, stdout.decode("utf-8", errors="replace"),
            stderr.decode("utf-8", errors="replace"),
        )
    except subprocess.TimeoutExpired as e:
        raise MicroVmTimeoutError(f"MicroVM execution timed out after {timeout} seconds") from e
