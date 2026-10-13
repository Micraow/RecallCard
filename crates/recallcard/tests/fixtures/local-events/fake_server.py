"""原创纯合成测试提供器；固定向量只证明真实HTTP/存储接线，不代表语义质量。"""
import json
from http.server import BaseHTTPRequestHandler, HTTPServer

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_POST(self):
        size=int(self.headers['Content-Length'])
        request=json.loads(self.rfile.read(size))
        vectors=[]
        for value in request['input']:
            vectors.append([1.0,0.0] if 'synthetic-alpha' in value or value=='test-query-a' else [0.0,1.0])
        data=json.dumps({'model':'synthetic-contract','data':[{'index':i,'embedding':v} for i,v in enumerate(vectors)]}).encode()
        self.send_response(200); self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)

server=HTTPServer(('127.0.0.1',0),Handler)
print(server.server_port,flush=True)
server.serve_forever()
