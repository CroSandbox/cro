# `libkrun-microvm` Python SDK

Serverless, hardware-isolated Python task execution on macOS (Hypervisor.framework) and Linux (KVM) using ephemeral libkrun microVMs.

## Features

- **`@task` Decorator**: Seamlessly offload any Python function to execute in an ephemeral, hardware-isolated microVM.
- **Host-Enforced Egress Proxy**: Restrict outbound network connections to explicit endpoints (`allow_hosts=["api.openai.com:443"]`), preventing data exfiltration and blocking SSRF to cloud metadata (`169.254.169.254`).
- **Zero-Trust Secret Substitution**: Keep raw API keys out of guest memory (`secrets={"OPENAI_API_KEY": "env:OPENAI_API_KEY"}`). Guest sees `krun-secret:OPENAI_API_KEY`, which is substituted in-flight by the host proxy.
- **LLM Token Metering & Hard Budgets**: Enforce strict token ceilings (`max_tokens=50000`). Once exceeded, outbound requests are blocked with `429 Too Many Requests`.

## Quickstart

```python
from libkrun_microvm import task

@task(
    image="python:3.11-slim",
    cpus=2,
    memory_mb=512,
    allow_hosts=["api.openai.com:443"],
    secrets={"OPENAI_API_KEY": "env:OPENAI_API_KEY"},
    max_tokens=10000,
)
def run_secure_agent(user_prompt: str) -> dict:
    import os, urllib.request, json

    # Note: In the microVM, os.environ["OPENAI_API_KEY"] is "krun-secret:OPENAI_API_KEY"
    # The host proxy intercepts outgoing HTTPS traffic to api.openai.com and substitutes
    # the true secret value in-flight, so guest code or memory dumps never reveal the secret!
    return {
        "status": "completed",
        "prompt": user_prompt,
    }

# Execute function inside isolated microVM
result = run_secure_agent("Analyze market data")
print("Result from microVM:", result)
```

## Result protocol and trust boundary

Guest results use base64-encoded JSON, limited to 8 MiB after decoding. Return
JSON-compatible values (objects, arrays, strings, finite numbers, booleans, or
null); arbitrary Python objects and legacy pickle results are rejected. The host
never unpickles guest output. Function and argument serialization into the guest
still uses pickle and requires trusted host callables and an importable function
definition in the guest image.

Captured stdout and stderr share a 16 MiB limit. Excess output terminates the
CLI and raises `MicroVmExecutionError`; configured timeouts still apply.
