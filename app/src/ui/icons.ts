const open = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">`;

export const icons = {
  recent: `${open}<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>`,
  library: `${open}<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>`,
  settings: `${open}<path d="M4 6h8M18 6h2M4 12h2M12 12h8M4 18h10M20 18h0"/><circle cx="15" cy="6" r="2.2"/><circle cx="9" cy="12" r="2.2"/><circle cx="17" cy="18" r="2.2"/></svg>`,
  rename: `${open}<path d="M4 20l1-4L16 5a2.1 2.1 0 0 1 3 3L8 19z"/><path d="M14 7l3 3"/></svg>`,
  trash: `${open}<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/></svg>`,
  folder: `${open}<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>`,
  record: `${open}<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="3.2" fill="currentColor" stroke="none"/></svg>`,
  play: `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 5.5v13a1 1 0 0 0 1.5.86l10.5-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z"/></svg>`,
  close: `${open}<path d="M6 6l12 12M18 6L6 18"/></svg>`,
  forge: `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M13.2 2.5c.3 3-1.2 4.3-2.6 5.8C9.2 9.8 8 11.2 8 13.4a4 4 0 0 0 8 0c0-1.2-.5-2.2-1.2-3 1.9.8 4.2 3 4.2 6.1A7 7 0 0 1 5 16.5c0-3 1.4-5 3-6.8 1.7-1.9 4.6-3.6 5.2-7.2z"/></svg>`,
} as const;
