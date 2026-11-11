// The playground's front end.
//
// One request, one reply: the program goes to `POST /run`, and the reply is
// either the program's output or the reason it was refused. There is no
// build step and no framework, because a playground that needs a toolchain to
// build is not a playground.

const source = document.getElementById('source');
const stdout = document.getElementById('stdout');
const stderr = document.getElementById('stderr');
const status = document.getElementById('status');
const run = document.getElementById('run');

async function send(program) {
  status.textContent = 'running...';
  stdout.textContent = '';
  stderr.textContent = '';
  try {
    const response = await fetch('/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ source: program, args: [] }),
    });
    const document_ = await response.json();
    if (document_.ok) {
      stdout.textContent = document_.stdout;
      stderr.textContent = document_.stderr;
      status.textContent = document_.exit === 0 ? 'exited 0' : `exited ${document_.exit}`;
    } else {
      // A refusal is not a crash: the server said why, and the front end
      // shows it.
      stderr.textContent = document_.message;
      status.textContent = `refused (${document_.status})`;
    }
  } catch (error) {
    stderr.textContent = String(error);
    status.textContent = 'the server did not answer';
  }
}

run.addEventListener('click', () => send(source.value));

// Ctrl-Enter runs, because a reader who has just changed a line wants to see
// the result now.
source.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    send(source.value);
  }
});
