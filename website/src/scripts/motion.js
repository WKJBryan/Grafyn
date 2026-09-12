const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

function setupNav() {
  const pill = document.querySelector('[data-nav]');
  if (!pill) return;
  const onScroll = () => {
    pill.classList.toggle('is-scrolled', window.scrollY > 24);
  };
  onScroll();
  window.addEventListener('scroll', onScroll, { passive: true });
}

function setupReveals() {
  const items = document.querySelectorAll('[data-reveal], [data-reveal-group]');
  if (!items.length) return;
  if (reduceMotion.matches) {
    items.forEach((el) => el.classList.add('is-in'));
    return;
  }
  const io = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        entry.target.classList.add('is-in');
        io.unobserve(entry.target);
      });
    },
    { threshold: 0.18 },
  );
  items.forEach((el) => io.observe(el));
}

// Scroll-linked, user-caused only: the hero art lags the scroll slightly.
// No idle animation; stops past the first viewport.
function setupHeroDrift() {
  const wrap = document.querySelector('[data-hero-drift]');
  if (!wrap || reduceMotion.matches) return;
  let latest = 0;
  let ticking = false;
  const update = () => {
    const offset = Math.min(latest * 0.1, 40);
    wrap.style.setProperty('--hero-y', `${-offset}px`);
    ticking = false;
  };
  window.addEventListener(
    'scroll',
    () => {
      latest = window.scrollY;
      if (!ticking) {
        ticking = true;
        requestAnimationFrame(update);
      }
    },
    { passive: true },
  );
}

setupNav();
setupReveals();
setupHeroDrift();
