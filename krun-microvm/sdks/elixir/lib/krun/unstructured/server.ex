# Copyright 2026, libkrun-sdk authors.
# SPDX-License-Identifier: Apache-2.0

defmodule Krun.Unstructured.Server do
  @moduledoc """
  High-performance concurrent Elixir OTP HTTP Server for Unstructured Document Intake,
  serving the Cyber-Obsidian Web UI at `/unstructured` and JSON REST API endpoints.
  """

  use GenServer
  require Logger

  @default_port 4000
  @max_header_bytes 16 * 1024
  @max_body_bytes 1024 * 1024

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl true
  def init(opts) do
    port = Keyword.get(opts, :port, @default_port)

    # Locate WebUI assets
    webui_dir =
      Keyword.get(opts, :webui_dir) ||
        find_webui_dir()

    case :gen_tcp.listen(port, [
           :binary,
           ip: {127, 0, 0, 1},
           packet: :raw,
           active: false,
           reuseaddr: true,
           send_timeout: 5000,
           send_timeout_close: true
         ]) do
      {:ok, listen_socket} ->
        {:ok, {_address, port}} = :inet.sockname(listen_socket)

        Logger.info(
          "[Krun.Unstructured] HTTP Server listening on http://127.0.0.1:#{port}/unstructured"
        )

        # Spawn acceptor loop
        spawn_link(fn -> acceptor_loop(listen_socket, webui_dir) end)
        {:ok, %{listen_socket: listen_socket, port: port, webui_dir: webui_dir}}

      {:error, reason} ->
        Logger.error("[Krun.Unstructured] Failed to bind port #{port}: #{inspect(reason)}")
        {:stop, reason}
    end
  end

  defp find_webui_dir do
    Path.expand("../../../../../../webui", __DIR__)
  end

  defp acceptor_loop(listen_socket, webui_dir) do
    case :gen_tcp.accept(listen_socket) do
      {:ok, client_socket} ->
        handler =
          spawn(fn ->
            receive do
              {:serve, socket} -> handle_client(socket, webui_dir)
            after
              5000 -> :ok
            end
          end)

        case :gen_tcp.controlling_process(client_socket, handler) do
          :ok -> send(handler, {:serve, client_socket})
          {:error, _} -> :gen_tcp.close(client_socket)
        end

        acceptor_loop(listen_socket, webui_dir)

      {:error, :closed} ->
        :ok

      {:error, reason} ->
        Logger.warning("[Krun.Unstructured] Accept error: #{inspect(reason)}")
        acceptor_loop(listen_socket, webui_dir)
    end
  end

  defp handle_client(socket, webui_dir) do
    deadline = System.monotonic_time(:millisecond) + 5000

    try do
      response =
        case read_request(socket, "", deadline) do
          {:ok, {method, path, headers, body}} ->
            {:ok, {_address, port}} = :inet.sockname(socket)
            host = headers["host"]
            origin = headers["origin"]
            content_type = headers["content-type"] || ""

            cond do
              host not in ["localhost:#{port}", "127.0.0.1:#{port}"] ->
                text_response(400, "Invalid Host header")

              origin != nil and origin != "http://#{host}" ->
                text_response(403, "Origin denied")

              method == "POST" and
                  String.downcase(hd(String.split(content_type, ";"))) != "application/json" ->
                text_response(415, "Expected application/json")

              true ->
                route(method, path, headers, body, webui_dir)
            end

          {:error, :too_large} ->
            text_response(413, "Request too large")

          {:error, _} ->
            text_response(400, "Invalid or incomplete request")
        end

      :gen_tcp.send(socket, response)
    rescue
      _ -> :gen_tcp.send(socket, text_response(400, "Invalid request"))
    after
      :gen_tcp.close(socket)
    end
  end

  defp recv_before(socket, deadline) do
    remaining = deadline - System.monotonic_time(:millisecond)
    if remaining > 0, do: :gen_tcp.recv(socket, 0, remaining), else: {:error, :timeout}
  end

  defp read_request(socket, data, deadline) do
    case :binary.match(data, "\r\n\r\n") do
      {offset, 4} when offset <= @max_header_bytes ->
        head = binary_part(data, 0, offset)
        body = binary_part(data, offset + 4, byte_size(data) - offset - 4)

        with {:ok, method, path, headers} <- parse_headers(head),
             {:ok, length} <- content_length(headers),
             {:ok, body} <- read_body(socket, body, length, deadline) do
          {:ok, {method, path, headers, body}}
        end

      :nomatch when byte_size(data) <= @max_header_bytes ->
        with {:ok, chunk} <- recv_before(socket, deadline),
             do: read_request(socket, data <> chunk, deadline)

      _ ->
        {:error, :too_large}
    end
  end

  defp read_body(_socket, _body, length, _deadline) when length > @max_body_bytes,
    do: {:error, :too_large}

  defp read_body(_socket, body, length, _deadline) when byte_size(body) >= length,
    do: {:ok, binary_part(body, 0, length)}

  defp read_body(socket, body, length, deadline) do
    with {:ok, chunk} <- recv_before(socket, deadline),
         do: read_body(socket, body <> chunk, length, deadline)
  end

  defp parse_headers(head) do
    [request_line | lines] = String.split(head, "\r\n")

    with [method, path, "HTTP/1.1"] <- String.split(request_line, " "),
         true <- method in ["GET", "POST"] and String.starts_with?(path, "/") do
      Enum.reduce_while(lines, {:ok, method, path, %{}}, fn line, {:ok, m, p, headers} ->
        case String.split(line, ":", parts: 2) do
          [name, value] ->
            name = String.downcase(name)

            if name == "" or Map.has_key?(headers, name) or name != String.trim(name) do
              {:halt, {:error, :invalid_headers}}
            else
              {:cont, {:ok, m, p, Map.put(headers, name, String.trim(value))}}
            end

          _ ->
            {:halt, {:error, :invalid_headers}}
        end
      end)
    else
      _ -> {:error, :invalid_request_line}
    end
  end

  defp content_length(headers) do
    value = headers["content-length"] || "0"

    if Map.has_key?(headers, "transfer-encoding") or not Regex.match?(~r/^[0-9]+$/, value) do
      {:error, :invalid_length}
    else
      {:ok, String.to_integer(value)}
    end
  end

  # =========================================================================
  # HTTP Routing & Handlers
  # =========================================================================

  # Root redirect to /unstructured
  defp route("GET", "/", _headers, _body, _webui_dir) do
    redirect_response("/unstructured")
  end

  # /unstructured: Main Cyber-Obsidian Web UI
  defp route("GET", path, _headers, _body, webui_dir)
       when path in ["/unstructured", "/unstructured/"] do
    index_file = Path.join(webui_dir, "index.html")

    case File.read(index_file) do
      {:ok, html} ->
        html_response(200, html)

      {:error, _} ->
        text_response(404, "WebUI index.html not found in #{webui_dir}")
    end
  end

  # CSS Asset
  defp route("GET", path, _headers, _body, webui_dir)
       when path in ["/index.css", "/unstructured/index.css"] do
    css_file = Path.join(webui_dir, "index.css")

    case File.read(css_file) do
      {:ok, css} ->
        file_response(200, "text/css; charset=utf-8", css)

      {:error, _} ->
        text_response(404, "CSS asset not found")
    end
  end

  # JS Asset
  defp route("GET", path, _headers, _body, webui_dir)
       when path in ["/app.js", "/unstructured/app.js"] do
    js_file = Path.join(webui_dir, "app.js")

    case File.read(js_file) do
      {:ok, js} ->
        file_response(200, "application/javascript; charset=utf-8", js)

      {:error, _} ->
        text_response(404, "JS asset not found")
    end
  end

  # API: Health check
  defp route("GET", "/api/unstructured/health", _headers, _body, _webui_dir) do
    json_response(200, %{
      status: "ok",
      engine: "libkrun-sdk",
      hypervisor: "Apple Silicon HVF / Linux KVM",
      egress_proxy: "Cloudflare Pingora 0.9.0 L7",
      page_path: "/unstructured",
      timestamp: System.system_time(:second)
    })
  end

  # API: Document Partitioning
  defp route("POST", "/api/unstructured/partition", _headers, body, _webui_dir) do
    case parse_json(body) do
      {:ok, params} ->
        doc = params["document"] || params["text"] || ""
        filename = params["filename"] || "document.txt"

        {:ok, elements} = Krun.Unstructured.partition(doc, filename: filename)

        json_response(200, %{
          status: "ok",
          filename: filename,
          elements_count: length(elements),
          elements: elements
        })

      {:error, _} ->
        json_response(400, %{status: "error", message: "Invalid JSON body"})
    end
  end

  # API: AI Threat Scan
  defp route("POST", "/api/unstructured/scan", _headers, body, _webui_dir) do
    case parse_json(body) do
      {:ok, params} ->
        doc = params["document"] || params["text"] || ""
        filename = params["filename"] || "document.txt"

        {:ok, result} = Krun.Unstructured.scan_threats(doc, filename: filename)
        json_response(200, %{status: "ok", result: result})

      {:error, _} ->
        json_response(400, %{status: "error", message: "Invalid JSON body"})
    end
  end

  # API: MicroVM Detonation & Sanitization
  defp route("POST", "/api/unstructured/detonate", _headers, _body, _webui_dir) do
    json_response(501, %{
      status: "error",
      message:
        "Hardware-isolated document detonation is not implemented. Use the text scan endpoint for heuristic analysis only.",
      isolation_verified: false
    })
  end

  # 404 Not Found fallback
  defp route(_method, path, _headers, _body, _webui_dir) do
    json_response(404, %{
      error: "NotFound",
      message: "Path '#{path}' not found. Did you mean '/unstructured'?",
      hint: "Visit http://127.0.0.1:4000/unstructured"
    })
  end

  # =========================================================================
  # Response Builders
  # =========================================================================

  defp redirect_response(location) do
    """
    HTTP/1.1 302 Found\r
    Location: #{location}\r
    Content-Length: 0\r
    Connection: close\r
    \r
    """
  end

  defp html_response(code, html) do
    bytes = byte_size(html)

    """
    HTTP/1.1 #{code} OK\r
    Content-Type: text/html; charset=utf-8\r
    Content-Length: #{bytes}\r
    X-Powered-By: libkrun-elixir\r
    Connection: close\r
    \r
    #{html}
    """
  end

  defp file_response(code, content_type, body) do
    bytes = byte_size(body)

    """
    HTTP/1.1 #{code} OK\r
    Content-Type: #{content_type}\r
    Content-Length: #{bytes}\r
    X-Powered-By: libkrun-elixir\r
    Connection: close\r
    \r
    #{body}
    """
  end

  defp text_response(code, text) do
    bytes = byte_size(text)

    """
    HTTP/1.1 #{code} Error\r
    Content-Type: text/plain; charset=utf-8\r
    Content-Length: #{bytes}\r
    Connection: close\r
    \r
    #{text}
    """
  end

  defp json_response(code, data) do
    json = JSON.encode!(data)
    bytes = byte_size(json)

    """
    HTTP/1.1 #{code} OK\r
    Content-Type: application/json; charset=utf-8\r
    Content-Length: #{bytes}\r
    X-Powered-By: libkrun-elixir\r
    Connection: close\r
    \r
    #{json}
    """
  end

  defp parse_json(body) do
    try do
      case JSON.decode!(body) do
        value when is_map(value) -> {:ok, value}
        _ -> {:error, :invalid_json}
      end
    rescue
      _ -> {:error, :invalid_json}
    end
  end
end
