const open = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">`;

export const icons = {
  recent: `${open}<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>`,
  library: `${open}<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>`,
  settings: `${open}<path d="M4 6h8M18 6h2M4 12h2M12 12h8M4 18h10M20 18h0"/><circle cx="15" cy="6" r="2.2"/><circle cx="9" cy="12" r="2.2"/><circle cx="17" cy="18" r="2.2"/></svg>`,
  rename: `${open}<path d="M4 20l1-4L16 5a2.1 2.1 0 0 1 3 3L8 19z"/><path d="M14 7l3 3"/></svg>`,
  trash: `${open}<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/></svg>`,
  folder: `${open}<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>`,
  mic: `${open}<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/></svg>`,
  record: `${open}<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="3.2" fill="currentColor" stroke="none"/></svg>`,
  play: `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 5.5v13a1 1 0 0 0 1.5.86l10.5-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z"/></svg>`,
  film: `${open}<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 4v16M17 4v16M3 9h4M3 15h4M17 9h4M17 15h4"/></svg>`,
  speaker: `${open}<path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5z"/><path d="M15.5 9a4.2 4.2 0 0 1 0 6M18 6.5a8 8 0 0 1 0 11"/></svg>`,
  keyboard: `${open}<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6.5 10h.01M10 10h.01M13.5 10h.01M17 10h.01M7 14h10"/></svg>`,
  cut: `${open}<circle cx="6" cy="6" r="2.6"/><circle cx="6" cy="18" r="2.6"/><path d="M8 7.8L20 18M8 16.2L20 6"/></svg>`,
  pause: `<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="5" width="4" height="14" rx="1"/></svg>`,
  back: `${open}<path d="M19 12H5M11 6l-6 6 6 6"/></svg>`,
  loop: `${open}<path d="M17 2l3 3-3 3"/><path d="M4 11V9a4 4 0 0 1 4-4h12"/><path d="M7 22l-3-3 3-3"/><path d="M20 13v2a4 4 0 0 1-4 4H4"/></svg>`,
  stepBack: `${open}<path d="M6 5v14"/><path d="M18 6l-8 6 8 6z" fill="currentColor"/></svg>`,
  stepForward: `${open}<path d="M18 5v14"/><path d="M6 6l8 6-8 6z" fill="currentColor"/></svg>`,
  close: `${open}<path d="M6 6l12 12M18 6L6 18"/></svg>`,
  forge: `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M13.2 2.5c.3 3-1.2 4.3-2.6 5.8C9.2 9.8 8 11.2 8 13.4a4 4 0 0 0 8 0c0-1.2-.5-2.2-1.2-3 1.9.8 4.2 3 4.2 6.1A7 7 0 0 1 5 16.5c0-3 1.4-5 3-6.8 1.7-1.9 4.6-3.6 5.2-7.2z"/></svg>`,
} as const;
