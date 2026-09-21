const modeData = {
  single: {
    number: '01',
    title: 'Fresh network namespace',
    description: 'A clean Linux network view with loopback brought up automatically. Perfect for testing bind behavior, local protocols, and isolation assumptions.',
    code: '<span class="code-purple">Sandbox</span><span>::new()</span><span class="code-muted">.enter(|| {</span><br><span class="code-indent">your_test_body()</span><br><span class="code-muted">})</span>',
    topology: '<div class="topo-host">HOST PROCESS</div><div class="topo-arrow">fork ↘</div><div class="topo-box topo-box-main"><span class="topo-label">USER + NET NS</span><strong>test body</strong><small>lo · 127.0.0.1</small></div><div class="topo-badge">host root not required</div>'
  },
  chaos: {
    number: '02',
    title: 'Real kernel chaos',
    description: 'Attach tc netem to the sandbox loopback. Delay, jitter, loss, reordering, corruption, and duplication are configured before the body runs.',
    code: '<span class="code-purple">Sandbox</span><span>::new()</span><span class="code-muted">.with_netem(Netem {</span><br><span class="code-indent">delay_ms: 100, loss_percent: 1.0,</span><br><span class="code-muted">}).enter(|| { ... })</span>',
    topology: '<div class="topo-host">LOOPBACK</div><div class="topo-arrow">netem ↘</div><div class="topo-box topo-box-main topo-box-chaos"><span class="topo-label">TC NETEM</span><strong>127.0.0.1</strong><small>100ms · 1% loss</small></div><div class="topo-badge">kernel-enforced impairment</div>'
  },
  paired: {
    number: '03',
    title: 'Two namespaces / one real link',
    description: 'A real veth pair carries traffic between separately isolated namespaces, with a readiness handshake before either closure runs.',
    code: '<span class="code-purple">Sandbox</span><span>::paired()</span><span class="code-muted">.enter(|a, b| {</span><br><span class="code-indent">a.send_to(b, payload)</span><br><span class="code-muted">})</span>',
    topology: '<div class="pair-diagram"><div class="topo-box topo-box-small"><span class="topo-label">NS A</span><strong>10.200.100.1</strong><small>veth-a</small></div><div class="pair-wire"><span></span></div><div class="topo-box topo-box-small"><span class="topo-label">NS B</span><strong>10.200.100.2</strong><small>veth-b</small></div></div><div class="topo-badge">carrier confirmed before test</div>'
  }
};

const modeTabs = document.querySelectorAll('.mode-tab');
const modeNumber = document.querySelector('#mode-number');
const modeTitle = document.querySelector('#mode-title');
const modeDescription = document.querySelector('#mode-description');
const modeCode = document.querySelector('.mode-code');
const topology = document.querySelector('#topology-single');

function selectMode(mode) {
  const data = modeData[mode];
  if (!data) return;

  modeTabs.forEach((tab) => {
    const active = tab.dataset.mode === mode;
    tab.classList.toggle('active', active);
    tab.setAttribute('aria-selected', String(active));
  });

  modeNumber.textContent = data.number;
  modeTitle.textContent = data.title;
  modeDescription.textContent = data.description;
  modeCode.innerHTML = data.code;
  topology.innerHTML = data.topology;
}

modeTabs.forEach((tab) => {
  tab.addEventListener('click', () => selectMode(tab.dataset.mode));
});

const copyButton = document.querySelector('.copy-button');
const installCommand = document.querySelector('#install-command');

async function copyInstallCommand() {
  const command = installCommand.textContent;
  try {
    await navigator.clipboard.writeText(command);
  } catch {
    const selection = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(installCommand);
    selection.removeAllRanges();
    selection.addRange(range);
    document.execCommand('copy');
    selection.removeAllRanges();
  }

  copyButton.textContent = 'COPIED';
  window.setTimeout(() => { copyButton.textContent = 'COPY'; }, 1400);
}

copyButton?.addEventListener('click', copyInstallCommand);

const menuButton = document.querySelector('.menu-button');
const navLinks = document.querySelector('.nav-links');

menuButton?.addEventListener('click', () => {
  const open = navLinks.classList.toggle('open');
  menuButton.setAttribute('aria-expanded', String(open));
});

navLinks?.querySelectorAll('a').forEach((link) => {
  link.addEventListener('click', () => {
    navLinks.classList.remove('open');
    menuButton?.setAttribute('aria-expanded', 'false');
  });
});

const revealItems = document.querySelectorAll('.reveal');
if ('IntersectionObserver' in window) {
  const revealObserver = new IntersectionObserver((entries, observer) => {
    entries.forEach((entry) => {
      if (!entry.isIntersecting) return;
      entry.target.classList.add('is-visible');
      observer.unobserve(entry.target);
    });
  }, { threshold: 0.12 });

  revealItems.forEach((item) => revealObserver.observe(item));
} else {
  revealItems.forEach((item) => item.classList.add('is-visible'));
}
