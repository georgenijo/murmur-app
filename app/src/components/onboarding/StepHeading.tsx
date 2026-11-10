import type { LucideIcon } from 'lucide-react';

/** The existing sidebar waveform, enlarged without changing its geometry. */
export function MurmurMark() {
  return <span className="murmur-hero-mark"><span className="home-brand-mark" aria-hidden="true"><span /><span /><span /><span /></span></span>;
}

export function StepHeading({ icon: Icon, title, subtitle, brand = false }: { icon: LucideIcon; title: string; subtitle: string; brand?: boolean }) {
  return <header className="onboarding-heading"><div className="onboarding-icon">{brand ? <MurmurMark /> : <Icon aria-hidden="true" />}</div><h1 tabIndex={-1}>{title}</h1><p>{subtitle}</p></header>;
}
