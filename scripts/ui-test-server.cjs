// Local test fixture only. Production assets are embedded by Tauri.
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const types = { '/': ['index.html', 'text/html'], '/manager.js': ['manager.js', 'text/javascript'], '/style.css': ['style.css', 'text/css'] };
http.createServer((req, res) => {
  const asset = types[req.url];
  if (!asset) { res.writeHead(404).end(); return; }
  res.setHeader('Content-Type', `${asset[1]}; charset=utf-8`);
  res.end(fs.readFileSync(path.join(__dirname, '..', 'ui', asset[0])));
}).listen(4329, '127.0.0.1');
