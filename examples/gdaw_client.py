import argparse
import json
import socket
import threading


class GDAWError(RuntimeError):
    pass


class GDAWClient:
    def __init__(self, port=7801, timeout=30, max_reply_bytes=33554432):
        if not 1 <= int(port) <= 65535:
            raise ValueError("port must be between 1 and 65535")
        self.connection = socket.create_connection(("127.0.0.1", int(port)), timeout)
        self.stream = self.connection.makefile("rwb")
        self.max_reply_bytes = max_reply_bytes
        self.sequence = 0
        self.lock = threading.Lock()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def close(self):
        try:
            self.stream.close()
        finally:
            self.connection.close()

    def request(self, method, params=None):
        with self.lock:
            self.sequence += 1
            identifier = self.sequence
            message = {"id": identifier, "method": method, "params": {} if params is None else params}
            payload = json.dumps(message, ensure_ascii=False, allow_nan=False).encode("utf-8") + b"\n"
            self.stream.write(payload)
            self.stream.flush()
            line = self.stream.readline(self.max_reply_bytes + 1)
            if not line or not line.endswith(b"\n") or len(line) > self.max_reply_bytes:
                self.close()
                raise GDAWError("The connection closed or the reply exceeded the configured limit")
            reply = json.loads(line)
            if reply.get("id") != identifier:
                self.close()
                raise GDAWError("The reply does not match this request")
            if not reply.get("ok"):
                raise GDAWError(reply.get("error", "GDAW rejected the request"))
            return reply.get("result")

    def execute(self, command, params=None):
        return self.request("engine.execute", {"command": command, "params": {} if params is None else params})

    def commands(self, query=""):
        return self.request("engine.commands", {"filter": query})

    def session(self, full=False):
        return self.request("session.inspect", {"detail": "full" if full else "summary"})

    def batch(self, commands):
        return [self.execute(command, params) for command, params in commands]


def main():
    parser = argparse.ArgumentParser(description="Send a command to a locally running GDAW instance")
    parser.add_argument("--port", type=int, default=7801)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("command", nargs="?", default="engine.capabilities")
    parser.add_argument("params", nargs="?", default="{}")
    args = parser.parse_args()
    with GDAWClient(args.port, args.timeout) as client:
        print(json.dumps(client.execute(args.command, json.loads(args.params)), indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
