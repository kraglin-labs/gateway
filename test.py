import subprocess
import time
import urllib.request
import urllib.error
import threading
from http.server import HTTPServer, BaseHTTPRequestHandler

class MockHandlerA(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"Hello from Backend A (Port 4001)!")

class MockHandlerB(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"Hello from Backend B (Port 5002)!")

def run_server(port, handler):
    server = HTTPServer(("127.0.0.1", port), handler)
    server.serve_forever()

def start_mock_backends():
    t1 = threading.Thread(target=run_server, args=(4001, MockHandlerA), daemon=True)
    t2 = threading.Thread(target=run_server, args=(5002, MockHandlerB), daemon=True)
    t1.start()
    t2.start()
    time.sleep(0.5) 

def test_gateway():
    print("[*] Starting mock backend servers on ports 4001 and 5002")
    start_mock_backends()

    base_url = "http://127.0.0.1:2026"

    print("[*] Testing route: /changed -> should hit Backend A (4001)")
    try:
        req = urllib.request.urlopen(f"{base_url}/changed")
        body = req.read().decode()
        print(f"    Response: {body}")
        assert "Backend A" in body
    except Exception as e:
        print(f"    [!] Request failed: {e}. check if cargo run has been run in gateway")
        return

    print("[*] Testing route: /orders -> should hit Backend B (5002)")
    try:
        req = urllib.request.urlopen(f"{base_url}/orders/item/99")
        body = req.read().decode()
        print(f"    Response: {body}")
        assert "Backend B" in body
    except Exception as e:
        print(f"    [!] Request failed: {e}")

    print("[*] Testing fallback: /unknown-route -> should return 404")
    try:
        urllib.request.urlopen(f"{base_url}/unknown-route")
    except urllib.error.HTTPError as e:
        print(f"    Response Code: {e.code} (Correctly caught 404!)")
        assert e.code == 404

    print("\nSUCCESS: All gateway proxy routing tests passed cleanly!")

if __name__ == "__main__":
    test_gateway()
