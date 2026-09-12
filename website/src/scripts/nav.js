const nav = document.querySelector('[data-nav]');
if (nav) {
  const burger = nav.querySelector('[data-burger]');
  const overlay = document.querySelector('[data-nav-overlay]');
  const links = overlay?.querySelectorAll('a') ?? [];

  const setOpen = (open) => {
    burger?.classList.toggle('is-open', open);
    burger?.setAttribute('aria-expanded', String(open));
    burger?.setAttribute('aria-label', open ? 'Close menu' : 'Open menu');
    overlay?.classList.toggle('is-open', open);
    if (overlay) {
      overlay.inert = !open;
      overlay.setAttribute('aria-hidden', String(!open));
    }
    document.body.style.overflow = open ? 'hidden' : '';
    if (open) {
      links.forEach((link, index) => {
        link.style.transitionDelay = `${80 + index * 50}ms`;
        link.classList.add('is-shown');
      });
    } else {
      links.forEach((link) => {
        link.style.transitionDelay = '0ms';
        link.classList.remove('is-shown');
      });
    }
  };

  burger?.addEventListener('click', () => {
    setOpen(!burger.classList.contains('is-open'));
  });

  overlay?.addEventListener('click', (event) => {
    if (event.target === overlay || event.target.closest('a')) setOpen(false);
  });

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') setOpen(false);
  });
}
