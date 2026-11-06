defmodule Krun.ServerSecurityTest do
  use ExUnit.Case

  setup do
    pid = start_supervised!({Krun.Unstructured.Server, [port: 0]})
    state = :sys.get_state(pid)
    %{port: state.port, listener: state.listen_socket}
  end

  defp connect(port) do
    {:ok, socket} = :gen_tcp.connect(~c"127.0.0.1", port, [:binary, active: false], 2000)
    socket
  end

  defp receive_all(socket, result \\ "") do
    case :gen_tcp.recv(socket, 0, 2000) do
      {:ok, data} -> receive_all(socket, result <> data)
      {:error, :closed} -> result
    end
  end

  defp request(port, text) do
    socket = connect(port)
    :ok = :gen_tcp.send(socket, text)
    receive_all(socket)
  end

  test "listener is restricted to loopback", %{listener: listener} do
    assert {:ok, {{127, 0, 0, 1}, _port}} = :inet.sockname(listener)
  end

  test "fragmented headers and bodies are assembled", %{port: port} do
    body = JSON.encode!(%{"document" => "hello"})
    socket = connect(port)
    :ok = :gen_tcp.send(socket, "POST /api/unstructured/scan HTTP/1.1\r\nHo")
    Process.sleep(20)

    :ok =
      :gen_tcp.send(
        socket,
        "st: 127.0.0.1:#{port}\r\nContent-Type: application/json\r\nContent-Length: #{byte_size(body)}\r\n\r\n"
      )

    Process.sleep(20)
    :ok = :gen_tcp.send(socket, body)
    response = receive_all(socket)
    assert response =~ "200 OK"
    assert response =~ ~s("is_threat":false)
  end

  test "oversized bodies and headers are rejected", %{port: port} do
    response =
      request(
        port,
        "POST /api/unstructured/scan HTTP/1.1\r\nHost: 127.0.0.1:#{port}\r\nContent-Length: 1048577\r\n\r\n"
      )

    assert response =~ "413"

    response =
      request(port, "GET / HTTP/1.1\r\nX-Padding: " <> String.duplicate("a", 17000) <> "\r\n\r\n")

    assert response =~ "413"
  end

  test "ambiguous framing is rejected", %{port: port} do
    for headers <- [
          "Content-Length: -1",
          "Content-Length: 1\r\nContent-Length: 2",
          "Transfer-Encoding: chunked"
        ] do
      response =
        request(
          port,
          "POST /api/unstructured/scan HTTP/1.1\r\nHost: 127.0.0.1:#{port}\r\n#{headers}\r\n\r\n"
        )

      assert response =~ "400"
    end
  end

  test "cross origin and DNS rebinding hosts are rejected", %{port: port} do
    response = request(port, "GET / HTTP/1.1\r\nHost: attacker.example:#{port}\r\n\r\n")
    assert response =~ "400"

    response =
      request(
        port,
        "POST /api/unstructured/scan HTTP/1.1\r\nHost: localhost:#{port}\r\nOrigin: https://attacker.example\r\n\r\n"
      )

    assert response =~ "403"
  end

  test "form submissions cannot invoke JSON APIs", %{port: port} do
    response =
      request(
        port,
        "POST /api/unstructured/scan HTTP/1.1\r\nHost: localhost:#{port}\r\nContent-Type: text/plain\r\n\r\n"
      )

    assert response =~ "415"
  end

  test "malformed requests close cleanly and leave server usable", %{port: port} do
    assert request(port, "garbage\r\n\r\n") =~ "400"

    assert request(
             port,
             "GET /api/unstructured/health HTTP/1.1\r\nHost: localhost:#{port}\r\n\r\n"
           ) =~ "200 OK"
  end

  test "JSON arrays cannot crash API handlers", %{port: port} do
    assert request(
             port,
             "POST /api/unstructured/scan HTTP/1.1\r\nHost: localhost:#{port}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n[]"
           ) =~ "400"
  end

  test "document strings cannot read host files", %{port: port} do
    path = Path.join(System.tmp_dir!(), "krun-secret-#{System.unique_integer([:positive])}")
    secret = "HOST_SECRET_MUST_NOT_LEAK"
    File.write!(path, secret)
    on_exit(fn -> File.rm(path) end)
    body = JSON.encode!(%{"document" => path})

    response =
      request(
        port,
        "POST /api/unstructured/partition HTTP/1.1\r\nHost: localhost:#{port}\r\nContent-Type: application/json\r\nContent-Length: #{byte_size(body)}\r\n\r\n#{body}"
      )

    refute response =~ secret
    assert response =~ path
  end

  test "detonation fails closed until hardware execution exists", %{port: port} do
    response =
      request(
        port,
        "POST /api/unstructured/detonate HTTP/1.1\r\nHost: localhost:#{port}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}"
      )

    assert response =~ "501"
    assert response =~ ~s("isolation_verified":false)
  end
end
