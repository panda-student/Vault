const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args);
const expected = { service: 'https://fixture-gemini.example.invalid/v1beta', account: 'fixture-user@example.com', password: 'FAKE_FILL_PASSWORD_LOCAL_123', api: 'FAKE_FILL_API_LOCAL_456', token: 'FAKE_FILL_TOKEN_LOCAL_789', ui: 'FAKE_UI_API_ACCEPTANCE_ONLY_123' };
let submitCount = 0, busy = false;
const prefixed = new Set();
document.getElementById('form').addEventListener('submit', event => { event.preventDefault(); submitCount++; });
setInterval(async () => {
  if (busy) return; busy = true;
  try {
    const command = await invoke('fixture_poll'); if (!command) return;
    const field = document.getElementById(command.field || 'button');
    if (command.action === 'focus' || command.action === 'prefix') {
      if (command.field !== 'readonly' && command.field !== 'button') { field.value = command.action === 'prefix' ? 'prefix-' : ''; if (command.action === 'prefix') prefixed.add(command.field); else prefixed.delete(command.field); }
      field.focus(); if (field.setSelectionRange) field.setSelectionRange(field.value.length, field.value.length);
      await new Promise(resolve => setTimeout(resolve, 200));
    }
    await invoke('fixture_ack', { ack: { seq: command.seq, matched: command.action === 'verify' ? field.value === (prefixed.has(command.field) ? 'prefix-' : '') + expected[command.field] : true, focused: document.hasFocus() && document.activeElement === field, submitCount } });
  } catch { /* Do not log field contents, exceptions or request objects. */ }
  finally { busy = false; }
}, 80);
