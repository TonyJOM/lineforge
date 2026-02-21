// SSE log streaming and session interaction

(function() {
  'use strict';

  // Session detail page
  if (typeof SESSION_ID !== 'undefined') {
    initSessionPage();
  }

  // New session form
  var newForm = document.getElementById('new-session-form');
  if (newForm) {
    initNewForm(newForm);
  }

  function initSessionPage() {
    var container = document.getElementById('terminal-container');
    var inputForm = document.getElementById('input-form');
    var inputText = document.getElementById('input-text');

    // Create xterm.js terminal matching PTY size (manager.rs hardcodes 24x80)
    var term = new Terminal({
      cols: 80,
      rows: 24,
      cursorBlink: true,
      scrollback: 5000,
      fontFamily: "'SF Mono', 'Cascadia Code', 'Fira Code', monospace",
      fontSize: 14,
      theme: {
        background: '#000000',
        foreground: '#e6edf3',
        cursor: '#58a6ff',
        selectionBackground: '#264f78',
        black: '#0d1117',
        red: '#f85149',
        green: '#3fb950',
        yellow: '#d29922',
        blue: '#58a6ff',
        magenta: '#bc8cff',
        cyan: '#39c5cf',
        white: '#e6edf3',
        brightBlack: '#8b949e',
        brightRed: '#f85149',
        brightGreen: '#3fb950',
        brightYellow: '#d29922',
        brightBlue: '#58a6ff',
        brightMagenta: '#bc8cff',
        brightCyan: '#39c5cf',
        brightWhite: '#ffffff'
      }
    });

    // Load fit addon to auto-size terminal to container
    var fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    term.open(container);
    fitAddon.fit();

    // Re-fit on window resize
    window.addEventListener('resize', function() {
      fitAddon.fit();
    });

    // Send keypresses directly to PTY via input endpoint
    term.onData(function(data) {
      fetch('/api/sessions/' + SESSION_ID + '/input', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text: data })
      });
    });

    // Connect SSE for log streaming
    var evtSource = new EventSource('/api/sessions/' + SESSION_ID + '/logs');
    var connected = false;

    evtSource.onopen = function() {
      if (connected) {
        term.reset();
      }
      connected = true;
    };

    evtSource.addEventListener('log', function(e) {
      try {
        var entry = JSON.parse(e.data);
        term.write(entry.data);
      } catch (err) {
        term.write(e.data);
      }
    });

    evtSource.addEventListener('gap', function(e) {
      term.write('\r\n--- ' + e.data + ' ---\r\n');
    });

    evtSource.onerror = function() {
      term.write('\r\n--- Connection lost, reconnecting... ---\r\n');
    };

    // Line-buffered input form as fallback
    inputForm.addEventListener('submit', function(e) {
      e.preventDefault();
      var text = inputText.value;
      if (!text && text !== '') return;

      fetch('/api/sessions/' + SESSION_ID + '/input', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text: text + '\n' })
      });

      inputText.value = '';
      inputText.focus();
    });

    // Focus terminal on click
    container.addEventListener('click', function() {
      term.focus();
    });
  }

  function initNewForm(form) {
    form.addEventListener('submit', function(e) {
      e.preventDefault();

      var body = {
        name: document.getElementById('name').value || undefined,
        tool: document.getElementById('tool').value,
        working_dir: document.getElementById('working_dir').value || undefined,
        auto_open_iterm: document.getElementById('auto_open_iterm').checked
      };

      fetch('/api/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body)
      })
      .then(function(resp) {
        if (resp.ok) return resp.json();
        throw new Error('Failed to create session');
      })
      .then(function(data) {
        window.location.href = '/sessions/' + data.id;
      })
      .catch(function(err) {
        alert(err.message);
      });
    });
  }
})();

// Global functions for button onclick handlers
function stopSession(id) {
  if (!confirm('Stop this session?')) return;
  fetch('/api/sessions/' + id + '/stop', { method: 'POST' })
    .then(function() { location.reload(); })
    .catch(function(err) { alert('Failed: ' + err.message); });
}

function openIterm(id) {
  fetch('/api/sessions/' + id + '/open-iterm', { method: 'POST' })
    .catch(function(err) { alert('Failed: ' + err.message); });
}
