// MUD Platform API helper — resolves paths relative to the area mount point.
// window.__MUD__ is injected by the platform at build time.
const mud = {
  get baseUrl() {
    return window.__MUD__?.baseUrl || './';
  },
  async fetch(path, options = {}) {
    const url = this.baseUrl + (path.startsWith('/') ? path.slice(1) : path);
    const res = await fetch(url, options);
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    return res;
  },
  async getJson(path) {
    return (await this.fetch(path)).json();
  },
  async postJson(path, body) {
    return (await this.fetch(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    })).json();
  }
};

const app = document.getElementById('app');
app.innerHTML = '<h1>MUD Area SPA</h1><p>Edit main.js to get started.</p>';

mud.getJson('api/status')
  .then(data => {
    const info = document.createElement('pre');
    info.textContent = JSON.stringify(data, null, 2);
    app.appendChild(info);
  })
  .catch(err => {
    const msg = document.createElement('p');
    msg.textContent = 'API not available: ' + err.message;
    msg.style.color = '#888';
    app.appendChild(msg);
  });
