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

// Story of the mark: the sticky figure advances (and rewinds) through
// seed → stem → graph as the text steps cross the viewport. The markup
// defaults to the fully-drawn stage, so with reduced motion or no JS the
// figure is simply complete.
function setupStory() {
  const figure = document.querySelector('[data-mark-story]');
  const steps = document.querySelectorAll('[data-story-step]');
  if (!figure || steps.length === 0 || reduceMotion.matches) return;
  figure.dataset.stage = '0';
  const io = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        figure.dataset.stage = entry.target.dataset.storyStep;
      });
    },
    { threshold: 0.5 },
  );
  steps.forEach((el) => io.observe(el));
}

// Scroll-linked, user-caused only: the layers hairline fills top-down as
// the section moves through the viewport.
function setupLayersLine() {
  const line = document.querySelector('[data-layers-line]');
  if (!line || reduceMotion.matches) return;
  const section = line.parentElement;
  let ticking = false;
  const update = () => {
    const rect = section.getBoundingClientRect();
    const progress = (window.innerHeight * 0.8 - rect.top) / rect.height;
    line.style.setProperty('--p', Math.min(1, Math.max(0, progress)).toFixed(4));
    ticking = false;
  };
  window.addEventListener(
    'scroll',
    () => {
      if (!ticking) {
        ticking = true;
        requestAnimationFrame(update);
      }
    },
    { passive: true },
  );
  update();
}

setupNav();
setupReveals();
setupStory();
setupLayersLine();
