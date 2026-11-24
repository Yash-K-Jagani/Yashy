import socket, sys, threading, time

port = int(sys.argv[1])
store = {}
srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(('127.0.0.1', port))
srv.listen(8)

def send_partial(conn, data):
    # split the reply so the client must assemble TCP chunks
    mid = max(1, len(data)//2)
    conn.sendall(data[:mid])
    time.sleep(0.01)
    conn.sendall(data[mid:])

def bulk(value):
    if value is None:
        return b'$-1\r\n'
    b = value if isinstance(value, bytes) else str(value).encode()
    return b'$' + str(len(b)).encode() + b'\r\n' + b + b'\r\n'

def handle(conn, round_no):
    buf = b''
    # reader for RESP arrays of bulk strings
    while True:
        while b'\r\n' not in buf:
            chunk = conn.recv(4096)
            if not chunk:
                conn.close(); return
            buf += chunk
        head, buf = buf.split(b'\r\n', 1)
        if not head.startswith(b'*'):
            conn.close(); return
        count = int(head[1:])
        parts = []
        while len(parts) < count:
            while b'\r\n' not in buf:
                chunk = conn.recv(4096)
                if not chunk:
                    conn.close(); return
                buf += chunk
            line, buf = buf.split(b'\r\n', 1)
            if not line.startswith(b'$'):
                conn.close(); return
            n = int(line[1:])
            while len(buf) < n + 2:
                chunk = conn.recv(4096)
                if not chunk:
                    conn.close(); return
                buf += chunk
            parts.append(buf[:n].decode())
            buf = buf[n+2:]
        cmd = parts[0].upper()
        if cmd == 'GET':
            reply = bulk(store.get(parts[1]))
        elif cmd == 'SET':
            store[parts[1]] = parts[2]
            reply = b'+OK\r\n'
        elif cmd == 'DEL':
            reply = b':1\r\n' if store.pop(parts[1], None) is not None else b':0\r\n'
        elif cmd == 'EXPIRE':
            reply = b':1\r\n' if parts[1] in store else b':0\r\n'
        elif cmd == 'PING':
            reply = b'+PONG\r\n'
        elif cmd == 'MULTI':
            reply = b'+OK\r\n'
        elif cmd == 'EXEC':
            reply = b'*2\r\n' + bulk('QUEUED') + bulk('DONE')
        elif cmd == 'PUBLISH':
            reply = b':1\r\n'
        elif cmd == 'SUBSCRIBE':
            reply = b'*3\r\n' + bulk('subscribe') + bulk(parts[1]) + b':1\r\n'
        elif cmd == 'QUIT':
            conn.close(); return
        else:
            reply = b'-ERR unknown command\r\n'
        send_partial(conn, reply)

count = 0
while True:
    conn, _ = srv.accept()
    count += 1
    threading.Thread(target=handle, args=(conn, count), daemon=True).start()
    # close the first connection after one command to exercise reconnect
    if count == 1:
        conn.settimeout(0.5)
