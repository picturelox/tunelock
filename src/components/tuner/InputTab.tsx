interface InputTabProps {
  icon: React.ReactNode;
  label: string;
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
}

export default function InputTab({ icon, label, active, disabled, onClick }: InputTabProps) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`
        flex items-center gap-2 px-4 py-2 rounded-md text-sm font-medium
        transition-colors
        ${active
          ? 'bg-accent-primary text-white'
          : 'bg-surface text-text-secondary hover:text-text-primary hover:bg-white/5'
        }
        ${disabled ? 'opacity-40 cursor-not-allowed' : 'cursor-pointer'}
      `}
    >
      {icon}
      {label}
    </button>
  );
}
