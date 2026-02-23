// SSE terminal streaming and native chat view

(function() {
  'use strict';

  if (typeof SESSION_ID !== 'undefined') {
    initSessionPage();
  }

  var newForm = document.getElementById('new-session-form');
  if (newForm) {
    initNewForm(newForm);
  }

  function initSessionPage() {
    var terminalApi = initTerminalView();
    var chatApi = initChatView();
    initViewSwitcher(terminalApi, chatApi);
  }

  function initViewSwitcher(terminalApi, chatApi) {
    var tabs = Array.prototype.slice.call(document.querySelectorAll('.view-tab'));
    var panes = Array.prototype.slice.call(document.querySelectorAll('.session-view'));

    function setView(view) {
      tabs.forEach(function(tab) {
        var active = tab.dataset.view === view;
        tab.classList.toggle('is-active', active);
        tab.setAttribute('aria-selected', active ? 'true' : 'false');
      });

      panes.forEach(function(pane) {
        var active = pane.dataset.viewPane === view;
        pane.classList.toggle('is-active', active);
      });

      if (view === 'terminal') {
        terminalApi.onShow();
      } else {
        chatApi.onShow();
      }
    }

    tabs.forEach(function(tab) {
      tab.addEventListener('click', function() {
        setView(tab.dataset.view);
      });
    });

    if ((SESSION_TOOL || '').toLowerCase() === 'claude') {
      setView('chat');
    } else {
      setView('terminal');
    }
  }

  function initTerminalView() {
    var container = document.getElementById('terminal-container');
    var inputForm = document.getElementById('terminal-input-form');
    var inputText = document.getElementById('terminal-input-text');

    var term = new Terminal({
      cursorBlink: true,
      scrollback: 5000,
      cols: 80,
      rows: 24,
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

    var fitAddon = null;
    if (typeof FitAddon !== 'undefined' && FitAddon.FitAddon) {
      fitAddon = new FitAddon.FitAddon();
      term.loadAddon(fitAddon);
    }

    term.open(container);
    if (fitAddon) {
      fitAddon.fit();
    }

    term.onData(function(data) {
      sendRawInput(data);
    });

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

    evtSource.addEventListener('resize', function(e) {
      try {
        var size = JSON.parse(e.data);
        term.resize(size.cols, size.rows);
        if (fitAddon) {
          fitAddon.fit();
        }
      } catch (err) {}
    });

    evtSource.onerror = function() {
      term.write('\r\n--- Connection lost, reconnecting... ---\r\n');
    };

    if (inputForm && inputText) {
      inputForm.addEventListener('submit', function(e) {
        e.preventDefault();
        var text = inputText.value;
        if (!text && text !== '') return;
        sendRawInput(text + '\n');
        inputText.value = '';
        inputText.focus();
      });
    }

    container.addEventListener('click', function() {
      term.focus();
    });

    window.addEventListener('resize', function() {
      if (fitAddon) {
        fitAddon.fit();
      }
    });

    return {
      onShow: function() {
        if (fitAddon) {
          fitAddon.fit();
        }
        term.focus();
      }
    };
  }

  function initChatView() {
    var root = document.getElementById('chat-view');
    var messagesEl = document.getElementById('chat-messages');
    var statusPill = document.getElementById('chat-status-pill');
    var modeButtons = Array.prototype.slice.call(document.querySelectorAll('.mode-btn'));
    var planEl = document.getElementById('chat-plan');
    var planItemsEl = document.getElementById('chat-plan-items');
    var planMarkdownEl = document.getElementById('chat-plan-markdown');
    var questionEl = document.getElementById('chat-question');
    var form = document.getElementById('chat-input-form');
    var input = document.getElementById('chat-input-text');

    var pollTimer = null;
    var lastSignature = '';

    var isClaudeSession = (SESSION_TOOL || '').toLowerCase() === 'claude';
    if (!isClaudeSession) {
      modeButtons.forEach(function(btn) { btn.disabled = true; });
      statusPill.textContent = 'Chat mode is only available for Claude sessions';
      input.disabled = true;
      form.querySelector('button[type="submit"]').disabled = true;
    }

    function pollChat(force) {
      if (!isClaudeSession) return;

      fetch('/api/sessions/' + SESSION_ID + '/chat')
        .then(function(resp) {
          if (!resp.ok) throw new Error('Failed to load chat');
          return resp.json();
        })
        .then(function(snapshot) {
          var signature = buildSignature(snapshot);
          if (!force && signature === lastSignature) {
            return;
          }
          lastSignature = signature;
          renderSnapshot(snapshot);
        })
        .catch(function(err) {
          statusPill.textContent = 'Chat unavailable';
          statusPill.className = 'chat-status-pill is-error';
          if (force) {
            console.error(err);
          }
        });
    }

    function buildSignature(snapshot) {
      var lastMessage = snapshot.messages && snapshot.messages.length
        ? snapshot.messages[snapshot.messages.length - 1]
        : null;
      return [
        snapshot.permission_mode,
        snapshot.view_mode,
        snapshot.state,
        snapshot.messages ? snapshot.messages.length : 0,
        lastMessage ? lastMessage.id + ':' + (lastMessage.text || '').length : 'none',
        snapshot.pending_question ? snapshot.pending_question.tool_use_id : 'noq',
        snapshot.plan ? snapshot.plan.items.join('|') : 'noplan',
        snapshot.plan && snapshot.plan.markdown ? snapshot.plan.markdown.length : 0
      ].join('::');
    }

    function renderSnapshot(snapshot) {
      statusPill.textContent = snapshot.status_label || 'Ready';
      statusPill.className = 'chat-status-pill state-' + (snapshot.state || 'idle');

      updateModeButtons(snapshot.view_mode || 'default');
      renderPlan(snapshot.plan);
      renderMessages(snapshot.messages || []);
      renderPendingQuestion(snapshot.pending_question || null);
    }

    function updateModeButtons(viewMode) {
      modeButtons.forEach(function(btn) {
        btn.classList.toggle('is-active', btn.dataset.chatMode === viewMode);
      });
    }

    function renderPlan(plan) {
      var hasItems = !!(plan && plan.items && plan.items.length);
      var hasMarkdown = !!(plan && plan.markdown);
      if (!hasItems && !hasMarkdown) {
        planEl.classList.add('hidden');
        planItemsEl.innerHTML = '';
        planMarkdownEl.textContent = '';
        return;
      }

      planEl.classList.remove('hidden');
      planItemsEl.innerHTML = '';
      if (hasItems) {
        plan.items.forEach(function(item) {
          var li = document.createElement('li');
          li.textContent = item;
          planItemsEl.appendChild(li);
        });
      }
      planItemsEl.classList.toggle('hidden', !hasItems);

      planMarkdownEl.textContent = hasMarkdown ? plan.markdown : '';
      planMarkdownEl.classList.toggle('hidden', !hasMarkdown);
    }

    function renderMessages(messages) {
      var atBottom = messagesEl.scrollHeight - messagesEl.scrollTop - messagesEl.clientHeight < 48;
      messagesEl.innerHTML = '';

      if (!messages.length) {
        var empty = document.createElement('div');
        empty.className = 'chat-empty';
        empty.textContent = 'No chat transcript yet. Send a message to Claude.';
        messagesEl.appendChild(empty);
      }

      messages.forEach(function(msg) {
        var row = document.createElement('div');
        row.className = 'chat-row role-' + msg.role + ' kind-' + msg.kind;

        var bubble = document.createElement('div');
        bubble.className = 'chat-bubble';

        var meta = document.createElement('div');
        meta.className = 'chat-meta';
        var label = labelForMessage(msg);
        meta.textContent = label + (msg.timestamp ? ' • ' + formatTime(msg.timestamp) : '');
        bubble.appendChild(meta);

        var body = document.createElement('div');
        body.className = 'chat-text';
        body.textContent = msg.text || '';
        bubble.appendChild(body);

        if (msg.is_error) {
          bubble.classList.add('is-error');
        }

        row.appendChild(bubble);
        messagesEl.appendChild(row);
      });

      if (atBottom) {
        messagesEl.scrollTop = messagesEl.scrollHeight;
      }
    }

    function renderPendingQuestion(question) {
      questionEl.innerHTML = '';
      if (!question || !question.questions || !question.questions.length) {
        questionEl.classList.add('hidden');
        return;
      }

      questionEl.classList.remove('hidden');

      question.questions.forEach(function(item) {
        var card = document.createElement('div');
        card.className = 'question-card';

        var header = document.createElement('div');
        header.className = 'question-header';
        header.textContent = item.header || 'Question';
        card.appendChild(header);

        var text = document.createElement('div');
        text.className = 'question-text';
        text.textContent = item.question || '';
        card.appendChild(text);

        var options = document.createElement('div');
        options.className = 'question-options';
        (item.options || []).forEach(function(option, idx) {
          var btn = document.createElement('button');
          btn.type = 'button';
          btn.className = 'question-option';
          btn.textContent = option.label + (option.description ? ' — ' + option.description : '');
          btn.addEventListener('click', function() {
            answerQuestion(option.label, idx + 1);
          });
          options.appendChild(btn);
        });
        card.appendChild(options);

        var freeform = document.createElement('form');
        freeform.className = 'question-freeform';
        freeform.innerHTML = '<input type="text" placeholder="Type your answer…" /><button type="submit" class="btn">Reply</button>';
        freeform.addEventListener('submit', function(e) {
          e.preventDefault();
          var value = freeform.querySelector('input').value;
          if (!value) return;
          answerQuestion(value, null);
          freeform.querySelector('input').value = '';
        });
        card.appendChild(freeform);

        questionEl.appendChild(card);
      });
    }

    function labelForMessage(msg) {
      if (msg.kind === 'thinking') return 'Claude (thinking)';
      if (msg.kind === 'tool_use') return msg.tool_name ? ('Tool: ' + msg.tool_name) : 'Tool';
      if (msg.kind === 'tool_result') return 'Tool result';
      if (msg.role === 'user') return 'You';
      if (msg.role === 'assistant') return 'Claude';
      return 'System';
    }

    function formatTime(iso) {
      try {
        return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
      } catch (err) {
        return iso;
      }
    }

    function answerQuestion(answer, optionIndex) {
      fetch('/api/sessions/' + SESSION_ID + '/chat/answer', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ answer: answer, option_index: optionIndex || undefined })
      })
      .then(function(resp) {
        if (!resp.ok) throw new Error('Failed to submit answer');
        pollChat(true);
      })
      .catch(function(err) {
        alert(err.message);
      });
    }

    modeButtons.forEach(function(btn) {
      btn.addEventListener('click', function() {
        if (!isClaudeSession) return;

        fetch('/api/sessions/' + SESSION_ID + '/chat/mode', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ mode: btn.dataset.chatMode })
        })
        .then(function(resp) {
          if (!resp.ok) throw new Error('Failed to switch mode');
          setTimeout(function() { pollChat(true); }, 300);
        })
        .catch(function(err) {
          alert(err.message);
        });
      });
    });

    form.addEventListener('submit', function(e) {
      e.preventDefault();
      var text = input.value.trim();
      if (!text) return;

      fetch('/api/sessions/' + SESSION_ID + '/chat/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text: text })
      })
      .then(function(resp) {
        if (!resp.ok) throw new Error('Failed to send message');
        input.value = '';
        pollChat(true);
      })
      .catch(function(err) {
        alert(err.message);
      });
    });

    pollChat(true);
    pollTimer = setInterval(function() { pollChat(false); }, 1200);

    return {
      onShow: function() {
        if (pollTimer == null) {
          pollTimer = setInterval(function() { pollChat(false); }, 1200);
        }
        pollChat(true);
        input.focus();
      }
    };
  }

  function sendRawInput(text) {
    fetch('/api/sessions/' + SESSION_ID + '/input', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text: text })
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
