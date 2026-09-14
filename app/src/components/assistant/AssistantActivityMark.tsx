export function AssistantActivityMark({ active }: { active: boolean }) {
  return <span className={`assistant-activity-mark ${active ? 'is-active' : 'is-settled'}`} aria-hidden="true">
    {Array.from({ length: 9 }, (_, index) => <span key={index} />)}
  </span>;
}
