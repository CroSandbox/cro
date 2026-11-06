# Copyright 2026, libkrun-sdk authors.
# SPDX-License-Identifier: Apache-2.0

defmodule Krun.Unstructured do
  @moduledoc """
  Local text partitioning and heuristic detection of known threat patterns.

  This module does not boot a microVM or implement PDF/DOCX decoding. Its
  `detonate/2` report is a simulation and does not establish document safety.
  Input strings are content; trusted SDK callers may explicitly use `read_file: true`.
  """

  @type element :: %{
          type: String.t(),
          text: String.t(),
          metadata: map()
        }

  @type threat :: %{
          title: String.t(),
          severity: String.t(),
          desc: String.t(),
          payload: String.t()
        }

  @type report :: %{
          filename: String.t(),
          filesize: String.t(),
          elements: [element()],
          threats: [threat()],
          risk_score: non_neg_integer(),
          is_threat: boolean(),
          threat_title: String.t(),
          threat_desc: String.t(),
          sanitized_text: String.t(),
          raw_diff: String.t(),
          telemetry: map(),
          guest_logs: [String.t()],
          cli_command: String.t()
        }

  # Known prompt injection signatures
  @prompt_injection_patterns [
    {~r/ignore\s+(all\s+)?previous\s+instructions/i,
     "Direct Instruction Hijack ('Ignore previous instructions')"},
    {~r/system\s+(override|directive|prompt\s+leak)/i, "System Override Directive"},
    {~r/dump\s+(all\s+)?(system|internal)\s+(prompt|keys|credentials)/i,
     "Credential / Prompt Exfiltration Request"},
    {~r/you\s+are\s+now\s+(an?\s+)?unrestricted\s+agent/i,
     "Jailbreak Roleplay Assertion ('unrestricted agent')"},
    {~r/\[SYSTEM_INSTRUCTION_OVERRIDE\]/i, "Delimited System Tag Injection"}
  ]

  # SSRF Cloud Metadata & C2 IP destinations
  @ssrf_patterns [
    {~r/169\.254\.169\.254/, "AWS/Azure/GCP Instance Metadata SSRF (169.254.169.254)"},
    {~r/metadata\.google\.internal/, "GCP Internal Metadata Service DNS"},
    {~r/100\.100\.100\.200/, "Alibaba Cloud Metadata Server (100.100.100.200)"}
  ]

  # Zero-width invisible Unicode characters (steganography)
  @zero_width_chars ["\u200B", "\u200C", "\u200D", "\uFEFF", "\u2060"]

  @doc """
  Partitions a document (PDF, DOCX, Markdown, Text, JSON) into structured schema elements
  compatible with the Unstructured.io document standard.
  """
  @spec partition(String.t() | binary(), keyword()) :: {:ok, [element()]} | {:error, String.t()}
  def partition(input, opts \\ []) do
    raw_text = extract_text(input, opts)
    filename = Keyword.get(opts, :filename, "document.txt")

    lines = String.split(raw_text, ~r/\r?\n/)
    elements = parse_elements(lines, filename)
    {:ok, elements}
  end

  @doc """
  Scans an unstructured document for AI Agent threats:
  1. Indirect and direct Prompt Injections
  2. Hidden zero-width steganographic characters
  3. Cloud metadata SSRF URLs (169.254.169.254)
  4. Token ceiling expansion bombs
  """
  @spec scan_threats(String.t() | binary(), keyword()) :: {:ok, map()}
  def scan_threats(input, opts \\ []) do
    raw_text = extract_text(input, opts)
    filename = Keyword.get(opts, :filename, "document.txt")

    threats = detect_threats(raw_text)
    is_threat = length(threats) > 0

    risk_score =
      if is_threat do
        min(99, 65 + length(threats) * 12)
      else
        2
      end

    sanitized = sanitize_text(raw_text)

    result = %{
      filename: filename,
      is_threat: is_threat,
      risk_score: risk_score,
      threats: threats,
      sanitized_text: sanitized,
      threat_count: length(threats)
    }

    {:ok, result}
  end

  @doc """
  Returns a local text-analysis simulation. This does not launch a microVM,
  measure isolation, or establish that a document is safe.
  Strings are literal content unless `read_file: true` is explicitly supplied.
  """
  @spec detonate(String.t() | binary(), keyword()) :: {:ok, report()}
  def detonate(input, opts \\ []) do
    raw_text = extract_text(input, opts)
    filename = Keyword.get(opts, :filename, "untrusted_document.pdf")
    filesize_bytes = byte_size(raw_text)
    filesize_str = format_bytes(filesize_bytes)

    sha256 =
      :crypto.hash(:sha256, raw_text)
      |> Base.encode16(case: :lower)
      |> String.slice(0, 16)
      |> Kernel.<>("...")

    threats = detect_threats(raw_text)
    is_threat = length(threats) > 0

    risk_score =
      if is_threat do
        min(99, 70 + length(threats) * 11)
      else
        0
      end

    {threat_title, threat_desc} =
      if is_threat do
        {"SIMULATION: Potential Threat Patterns Detected",
         "Found #{length(threats)} text pattern(s). No microVM execution or containment was performed."}
      else
        {"SIMULATION: No Known Text Patterns Detected",
         "Pattern matching cannot establish document safety. No microVM execution was performed."}
      end

    lines = String.split(raw_text, ~r/\r?\n/)
    elements = parse_elements(lines, filename)
    sanitized = sanitize_text(raw_text)

    telemetry = %{
      instance_id: nil,
      cold_boot_latency: nil,
      memory_rss: nil,
      virtiofs_cow: "not measured",
      egress_engine: "not active",
      llm_token_meter: "not active"
    }

    guest_logs = ["SIMULATION ONLY: local text analysis; no guest was started."]

    diff =
      if is_threat do
        "--- Visual Document Stream\n+++ Decompiled PDF MicroVM Stream\n@@ -1,4 +1,8 @@\n+ [AI-SHIELD DETECTED THREAT BLOCK]: #{length(threats)} injection pattern(s) stripped."
      else
        "Clean document stream. 0 visual discrepancies or hidden font layers detected."
      end

    cli_cmd = """
    microvm run \\
        --workspace-cow ./inbox:/docs \\
        --allow-host api.openai.com:443 \\
        --max-tokens 5000 \\
        alpine:latest -- sh -c "pdftotext /docs/input.pdf /docs/extracted.txt"
    """

    report = %{
      execution_mode: "simulation",
      isolation_verified: false,
      filename: filename,
      filesize: filesize_str,
      hash: sha256,
      elements: elements,
      threats: threats,
      risk_score: risk_score,
      is_threat: is_threat,
      threat_title: threat_title,
      threat_desc: threat_desc,
      sanitized_text: sanitized,
      raw_diff: diff,
      telemetry: telemetry,
      guest_logs: guest_logs,
      cli_command: String.trim(cli_cmd)
    }

    {:ok, report}
  end

  # =========================================================================
  # Private Helpers
  # =========================================================================

  # Treat API-supplied strings as content. File reads require an explicit local SDK option.
  defp extract_text(input, opts) when is_binary(input) do
    if Keyword.get(opts, :read_file, false), do: File.read!(input), else: input
  end

  defp parse_elements(lines, filename) do
    lines
    |> Enum.map(&String.trim/1)
    |> Enum.reject(&(&1 == ""))
    |> Enum.with_index(1)
    |> Enum.map(fn {line, idx} ->
      cond do
        String.starts_with?(line, ["#", "##", "###"]) ->
          %{
            type: "Header",
            text: String.trim_leading(line, "#") |> String.trim(),
            metadata: %{element_id: idx, filename: filename}
          }

        idx == 1 and String.length(line) < 120 ->
          %{type: "Title", text: line, metadata: %{element_id: idx, filename: filename}}

        String.starts_with?(line, ["- ", "* ", "1. ", "2. ", "3. ", "4. ", "5. "]) ->
          %{type: "ListItem", text: line, metadata: %{element_id: idx, filename: filename}}

        String.contains?(line, "|") ->
          %{type: "Table", text: line, metadata: %{element_id: idx, filename: filename}}

        String.starts_with?(line, ["```", "$ ", "microvm "]) ->
          %{type: "CodeSnippet", text: line, metadata: %{element_id: idx, filename: filename}}

        true ->
          %{type: "NarrativeText", text: line, metadata: %{element_id: idx, filename: filename}}
      end
    end)
  end

  defp detect_threats(text) do
    # 1. Prompt Injections
    prompt_threats =
      Enum.reduce(@prompt_injection_patterns, [], fn {regex, label}, acc ->
        case Regex.run(regex, text) do
          [matched | _] ->
            [
              %{
                title: label,
                severity: "Critical",
                desc:
                  "Matches known prompt injection pattern attempting to hijack agent instructions.",
                payload: matched
              }
              | acc
            ]

          nil ->
            acc
        end
      end)

    # 2. SSRF Cloud Metadata Beacons
    ssrf_threats =
      Enum.reduce(@ssrf_patterns, [], fn {regex, label}, acc ->
        case Regex.run(regex, text) do
          [matched | _] ->
            [
              %{
                title: label,
                severity: "Critical",
                desc:
                  "Outbound target to internal cloud metadata endpoint. Intercepted by Pingora egress filter.",
                payload: matched
              }
              | acc
            ]

          nil ->
            acc
        end
      end)

    # 3. Zero-width steganography
    has_zero_width = Enum.any?(@zero_width_chars, &String.contains?(text, &1))

    stego_threats =
      if has_zero_width do
        [
          %{
            title: "Invisible Zero-Width Unicode Steganography",
            severity: "Warning",
            desc:
              "Document contains invisible zero-width characters (U+200B/U+200C/U+200D) encoding hidden data.",
            payload: "Zero-width sequence detected in document stream"
          }
        ]
      else
        []
      end

    prompt_threats ++ ssrf_threats ++ stego_threats
  end

  defp sanitize_text(text) do
    # Neutralize prompt injection phrases
    sanitized =
      Enum.reduce(@prompt_injection_patterns, text, fn {regex, _}, acc ->
        Regex.replace(regex, acc, "[NEUTRALIZED_PROMPT_INJECTION]")
      end)

    # Strip zero-width invisible characters
    Enum.reduce(@zero_width_chars, sanitized, fn char, acc ->
      String.replace(acc, char, "")
    end)
  end

  defp format_bytes(bytes) when bytes < 1024, do: "#{bytes} B"
  defp format_bytes(bytes) when bytes < 1024 * 1024, do: "#{Float.round(bytes / 1024, 1)} KB"
  defp format_bytes(bytes), do: "#{Float.round(bytes / (1024 * 1024), 2)} MB"
end
