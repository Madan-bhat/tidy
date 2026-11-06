(function () {
  "use strict";

  var root = document.documentElement;
  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  var wideHero = window.matchMedia("(min-width: 921px)");
  var animate = !reduceMotion.matches;

  // Expo-out: quick start, long soft landing — used for every programmatic scroll.
  function easeOutExpo(t) {
    return t === 1 ? 1 : 1 - Math.pow(2, -10 * t);
  }

  function clamp(v, min, max) {
    return Math.min(max, Math.max(min, v));
  }

  var lenis = null;
  if (window.Lenis) {
    lenis = new window.Lenis({
      autoRaf: false,
      lerp: 0.1,
      wheelMultiplier: 1,
      touchMultiplier: 1.4,
      smoothWheel: true,
      respectReducedMotion: true,
      stopInertiaOnNavigate: true,
      allowNestedScroll: true,
      anchors: { offset: -88, duration: 1.4, easing: easeOutExpo },
    });
    window.lenis = lenis;
  }

  function scrollTo(target, opts) {
    var el = typeof target === "string" ? document.querySelector(target) : target;
    if (lenis) {
      lenis.scrollTo(el || target, Object.assign({ duration: 1.4, easing: easeOutExpo }, opts));
    } else if (el && el.scrollIntoView) {
      el.scrollIntoView({ behavior: animate ? "smooth" : "auto", block: "start" });
    } else if (typeof target === "number") {
      window.scrollTo({ top: target, behavior: animate ? "smooth" : "auto" });
    }
  }

  // ---------- Elements ----------

  var progressBar = document.querySelector(".scroll-progress span");
  var navWrap = document.querySelector(".nav-wrap");
  var parallax = Array.prototype.slice.call(document.querySelectorAll("[data-speed]"));
  var band = document.querySelector(".band");
  var bandTrack = document.querySelector(".band-track");
  var baGrid = document.querySelector(".ba-grid");
  var baCount = document.querySelector(".ba-count");
  var messItems = baGrid ? Array.prototype.slice.call(baGrid.querySelectorAll(".mess [data-file]")) : [];
  var treeItems = baGrid ? Array.prototype.slice.call(baGrid.querySelectorAll(".tree [data-file]")) : [];
  var stepsSection = document.querySelector(".how");
  var stepsLine = document.querySelector(".steps-line span");
  var steps = Array.prototype.slice.call(document.querySelectorAll(".how .steps li"));
  var matchSteps = Array.prototype.slice.call(document.querySelectorAll(".match-steps li"));

  if (animate) root.classList.add("fx");
  if (animate && baGrid) baGrid.classList.add("scrub");
  if (animate && bandTrack) band.classList.add("live");

  // ---------- Scroll-driven updates ----------

  var lastY = -1;
  var navHidden = false;
  var filed = -1;

  function sectionProgress(el, startAt, endAt) {
    // 0 when el's top reaches startAt (fraction of viewport), 1 when its bottom reaches endAt.
    var r = el.getBoundingClientRect();
    var vh = window.innerHeight;
    var start = vh * startAt;
    var end = vh * endAt - r.height;
    return clamp((start - r.top) / (start - end), 0, 1);
  }

  function onScroll(y, direction, limit) {
    if (progressBar) {
      progressBar.style.transform = "scaleX(" + (limit > 0 ? y / limit : 0) + ")";
    }

    if (navWrap) {
      navWrap.classList.toggle("scrolled", y > 8);
      var hide = y > 240 && direction === 1;
      var show = direction === -1 || y <= 240;
      if (hide && !navHidden) {
        navHidden = true;
        navWrap.classList.add("hidden");
      } else if (show && navHidden) {
        navHidden = false;
        navWrap.classList.remove("hidden");
      }
    }

    if (!animate) return;

    // Parallax only matters while the hero is on screen, and only when hero columns sit
    // side by side — stacked, the card would drift into the marquee below it.
    var py = wideHero.matches ? Math.min(y, window.innerHeight * 1.2) : 0;
    for (var i = 0; i < parallax.length; i++) {
      var el = parallax[i];
      var speed = parseFloat(el.getAttribute("data-speed")) || 0;
      el.style.transform = "translate3d(0," + (py * speed).toFixed(2) + "px,0)";
    }

    if (baGrid && messItems.length) {
      var p = sectionProgress(baGrid, 0.85, 1.05);
      var n = Math.floor(p * (messItems.length + 0.999));
      if (n !== filed) {
        filed = n;
        var done = {};
        messItems.forEach(function (li, idx) {
          var isFiled = idx < n;
          li.classList.toggle("filed", isFiled);
          if (isFiled) done[li.getAttribute("data-file")] = true;
        });
        treeItems.forEach(function (t) {
          t.classList.toggle("in", !!done[t.getAttribute("data-file")]);
        });
        if (baCount) {
          var left = messItems.length - n;
          baCount.textContent = left === 0 ? "empty" : left + (left === 1 ? " file" : " files");
        }
      }
    }

    if (stepsSection && stepsLine) {
      var sp = sectionProgress(stepsSection, 0.75, 0.75);
      stepsLine.style.transform = "scaleX(" + sp + ")";
      steps.forEach(function (li, idx) {
        li.classList.toggle("passed", sp >= idx / steps.length + 0.02);
      });
    }

    if (matchSteps.length) {
      var mid = window.innerHeight * 0.5;
      var best = null;
      var bestDist = Infinity;
      matchSteps.forEach(function (li) {
        var r = li.getBoundingClientRect();
        var d = Math.abs(r.top + r.height / 2 - mid);
        if (d < bestDist) {
          bestDist = d;
          best = li;
        }
      });
      matchSteps.forEach(function (li) {
        li.classList.toggle("current", li === best);
      });
    }
  }

  function nativeLimit() {
    return Math.max(0, root.scrollHeight - window.innerHeight);
  }

  if (lenis) {
    lenis.on("scroll", function (l) {
      onScroll(l.scroll, l.direction, l.limit);
    });
  } else {
    window.addEventListener(
      "scroll",
      function () {
        var y = window.scrollY;
        var dir = lastY < 0 ? 0 : y > lastY ? 1 : y < lastY ? -1 : 0;
        lastY = y;
        onScroll(y, dir, nativeLimit());
      },
      { passive: true }
    );
  }

  // ---------- Single rAF loop: Lenis + velocity-driven marquee ----------

  var bandX = 0;
  var bandSkew = 0;
  var bandWidth = 0;
  var lastTime = 0;

  function measureBand() {
    var set = bandTrack && bandTrack.firstElementChild;
    bandWidth = set ? set.getBoundingClientRect().width : 0;
  }

  function frame(time) {
    var dt = lastTime ? Math.min(64, time - lastTime) : 16;
    lastTime = time;
    if (lenis) lenis.raf(time);

    if (animate && bandTrack && bandWidth) {
      var velocity = lenis ? lenis.velocity : 0;
      var dir = lenis && lenis.direction === -1 ? -1 : 1;
      // Base drift plus a boost proportional to how fast the page is moving.
      var speed = (0.045 + Math.min(Math.abs(velocity), 60) * 0.03) * dir;
      bandX -= speed * dt;
      if (bandX <= -bandWidth) bandX += bandWidth;
      if (bandX > 0) bandX -= bandWidth;
      bandSkew += (clamp(velocity * -0.12, -6, 6) - bandSkew) * 0.12;
      bandTrack.style.transform =
        "translate3d(" + bandX.toFixed(2) + "px,0,0) skewX(" + bandSkew.toFixed(2) + "deg)";
    }

    requestAnimationFrame(frame);
  }

  measureBand();
  window.addEventListener("resize", function () {
    measureBand();
    if (lenis) lenis.resize();
  });
  if (document.fonts && document.fonts.ready) {
    document.fonts.ready.then(function () {
      measureBand();
      if (lenis) lenis.resize();
    });
  }
  requestAnimationFrame(frame);

  // Paint the initial state (e.g. reload mid-page).
  onScroll(window.scrollY, 0, lenis ? lenis.limit : nativeLimit());

  // ---------- Programmatic scrolling ----------

  document.addEventListener("click", function (e) {
    var trigger = e.target.closest("[data-scroll-to]");
    if (!trigger) return;
    e.preventDefault();
    var target = trigger.getAttribute("data-scroll-to");
    scrollTo(target === "#top" ? 0 : target, { duration: 1.8 });
  });

  // Opening an FAQ answer near the bottom of the screen glides it into view.
  document.querySelectorAll(".faq-list details").forEach(function (d) {
    d.addEventListener("toggle", function () {
      if (!d.open) return;
      if (lenis) lenis.resize();
      var r = d.getBoundingClientRect();
      if (r.bottom > window.innerHeight - 24) {
        var offset = Math.min(-100, -(window.innerHeight - r.height - 48));
        scrollTo(d, { offset: offset, duration: 0.9 });
      }
    });
  });

  // Pause smooth scrolling while the tab is hidden so it doesn't resume mid-glide.
  document.addEventListener("visibilitychange", function () {
    if (!lenis) return;
    if (document.hidden) lenis.stop();
    else lenis.start();
  });

  reduceMotion.addEventListener &&
    reduceMotion.addEventListener("change", function () {
      window.location.reload();
    });
})();
