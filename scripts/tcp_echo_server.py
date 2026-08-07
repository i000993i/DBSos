#!/usr/bin/env python3
"""Мини-сервер для ручного теста TCP-стека DBSos.

Поднимает echo-сервер на хосте. Гостевая ОС (QEMU slirp) обращается к
10.0.2.2:<port> — slirp пробрасывает это на loopback хоста.

Запуск:
    python3 scripts/tcp_echo_server.py [port]

Поведение:
    - печатает полученный байты,
    - отправляет обратно "<ECHO> " + данные, подтверждая доставку.

Это позволяет из гостя выполнить:
    tcp 10.0.2.2 8000 hello
и увидеть ответ "<ECHO> hello".
"""
import socket
import sys


def main() -> None:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8000
    host = "127.0.0.1"
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, port))
    srv.listen(1)
    print(f"[echo] listening on {host}:{port} (client: 10.0.2.2:{port})", flush=True)
    try:
        while True:
            conn, addr = srv.accept()
            print(f"[echo] connection from {addr}", flush=True)
            with conn:
                while True:
                    data = conn.recv(4096)
                    if not data:
                        break
                    print(f"[echo] recv {len(data)}B: {data!r}", flush=True)
                    conn.sendall(b"<ECHO> " + data)
    except KeyboardInterrupt:
        print("\n[echo] stopped", flush=True)


if __name__ == "__main__":
    main()