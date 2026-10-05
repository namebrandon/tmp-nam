import type * as React from 'react';

export type IconName = 'captures' | 'tone3000' | 'sdcard' | 'settings' | 'warning' | 'error' | 'info' | 'usb' | 'add' | 'minus' | 'check' | 'close' | 'external' | 'bookmark' | 'refresh' | 'lock' | 'chevron';

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** secondary is the default. One primary per view. danger only on the button that erases or removes. */
  variant?: 'secondary' | 'primary' | 'danger' | 'plain';
  size?: 'sm' | 'md' | 'lg';
  icon?: IconName;
  /** Hide the label; pass aria-label. */
  iconOnly?: boolean;
  /** Red text on a secondary button, for "Remove from Unit…" style entry points. */
  destructiveText?: boolean;
  /** Force a visual state in docs and mockups. */
  state?: 'hover' | 'pressed' | 'focus';
}
export declare function Button(props: ButtonProps): React.ReactElement;

export interface IconProps { name: IconName; size?: number; strokeWidth?: number; 'aria-label'?: string; className?: string; style?: React.CSSProperties }
export declare function Icon(props: IconProps): React.ReactElement;

export interface TagProps { tone?: 'neutral' | 'arch' | 'ok' | 'warn' | 'danger' | 'accent'; children?: React.ReactNode; className?: string; style?: React.CSSProperties }
export declare function Tag(props: TagProps): React.ReactElement;

export interface StatusDotProps { tone?: 'ok' | 'warn' | 'danger' | 'off'; label?: React.ReactNode }
export declare function StatusDot(props: StatusDotProps): React.ReactElement;

export interface SpinnerProps { size?: 'sm' | 'lg'; label?: string }
export declare function Spinner(props: SpinnerProps): React.ReactElement;

export interface ProgressBarProps {
  /** 0–100. Omit for indeterminate. */
  value?: number;
  indeterminate?: boolean;
  tone?: 'accent' | 'ok' | 'danger';
  size?: 'md' | 'lg';
  label?: React.ReactNode;
  /** Right-aligned detail, e.g. "143 of 298 KB". */
  detail?: React.ReactNode;
}
export declare function ProgressBar(props: ProgressBarProps): React.ReactElement;

export interface BannerAction { label: string; onClick?: () => void; variant?: ButtonProps['variant']; disabled?: boolean }
export interface BannerProps {
  /** note = the blue "applies next time" style. */
  tone?: 'info' | 'note' | 'warn' | 'error' | 'ok';
  title?: React.ReactNode;
  children?: React.ReactNode;
  actions?: BannerAction[];
  onDismiss?: () => void;
  dismissLabel?: string;
  className?: string;
  style?: React.CSSProperties;
}
export declare function Banner(props: BannerProps): React.ReactElement;

export interface TextFieldProps extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'children'> {
  label?: React.ReactNode;
  mono?: boolean;
  /** true for the red outline only; a string also prints the message. */
  error?: boolean | string;
  help?: React.ReactNode;
  state?: 'focus';
  /** Buttons placed to the right of the input. */
  children?: React.ReactNode;
}
export declare function TextField(props: TextFieldProps): React.ReactElement;

export interface CheckboxProps { label?: React.ReactNode; checked?: boolean; defaultChecked?: boolean; disabled?: boolean; onChange?: React.ChangeEventHandler<HTMLInputElement>; 'aria-label'?: string }
export declare function Checkbox(props: CheckboxProps): React.ReactElement;

export interface RadioProps {
  label: React.ReactNode;
  detail?: React.ReactNode;
  /** Right-aligned reason, e.g. "Not a USB reader". */
  note?: React.ReactNode;
  name?: string;
  checked?: boolean;
  defaultChecked?: boolean;
  disabled?: boolean;
  /** List-row layout with divider and selected fill (SD card list). */
  row?: boolean;
  onChange?: React.ChangeEventHandler<HTMLInputElement>;
}
export declare function Radio(props: RadioProps): React.ReactElement;

export interface PopupButtonProps { children?: React.ReactNode; tone?: 'default' | 'warn'; open?: boolean; marked?: boolean; disabled?: boolean; onClick?: () => void; style?: React.CSSProperties }
export declare function PopupButton(props: PopupButtonProps): React.ReactElement;

export type MenuItem =
  | { type: 'header'; label: string }
  | { type: 'separator' }
  | { label: string; value?: string; checked?: boolean; disabled?: boolean; note?: string; link?: boolean; active?: boolean; onClick?: () => void };
export interface MenuProps { items: MenuItem[]; onSelect?: (value: string) => void; width?: number; 'aria-label'?: string; style?: React.CSSProperties }
export declare function Menu(props: MenuProps): React.ReactElement;

export interface SegmentedControlProps { options: (string | { label: string; value: string })[]; value?: string; onChange?: (value: string) => void; 'aria-label'?: string }
export declare function SegmentedControl(props: SegmentedControlProps): React.ReactElement;

export interface SizePickerProps {
  /** Smallest → largest, labels as the container names them. */
  sizes: (string | { label: string; note?: string })[];
  value?: string;
  onChange?: (label: string) => void;
  disabled?: boolean;
}
export declare function SizePicker(props: SizePickerProps): React.ReactElement;

export interface GainControlProps {
  /** 0–8 in 0.5 steps; 1 = as captured. */
  value?: number;
  /** Value before this edit; shows "was 1×". */
  previous?: number;
  onChange?: (value: number) => void;
  disabled?: boolean;
  label?: string;
  /** false hides the 12 dB help line. */
  help?: boolean;
}
export declare function GainControl(props: GainControlProps): React.ReactElement;

export interface Stage { label: string; state?: 'done' | 'now' | 'todo' | 'fail'; detail?: string }
export interface StageListProps { stages: Stage[] }
export declare function StageList(props: StageListProps): React.ReactElement;

export interface SheetAction { label: string; variant?: ButtonProps['variant']; onClick?: () => void; disabled?: boolean; autoFocus?: boolean }
export interface SheetProps {
  open?: boolean;
  title?: React.ReactNode;
  width?: number;
  /** alertdialog role for destructive confirmations. */
  alert?: boolean;
  /** Right-aligned buttons, primary last. */
  actions?: SheetAction[];
  /** Left-aligned plain button, e.g. "Add More Files…". */
  extra?: SheetAction;
  onClose?: () => void;
  children?: React.ReactNode;
}
export declare function Sheet(props: SheetProps): React.ReactElement | null;

export type UnitState = 'connected' | 'looking' | 'missing' | 'busy';
export interface Activity { title: string; detail?: string; /** 0–100; omit or -1 for indeterminate */ value?: number }
export interface SidebarProps {
  active?: 'captures' | 'tone3000' | 'sdcard' | 'settings';
  onNavigate?: (key: 'captures' | 'tone3000' | 'sdcard' | 'settings') => void;
  captureCount?: number | string;
  unit?: UnitState;
  /** A running long operation; shown on every page. */
  activity?: Activity | null;
  /** Icon-only, below 820 px window width. */
  compact?: boolean;
  style?: React.CSSProperties;
}
export declare function Sidebar(props: SidebarProps): React.ReactElement;

export interface UnitStatusProps { state?: UnitState; compact?: boolean }
export declare function UnitStatus(props: UnitStatusProps): React.ReactElement;

export interface ActivityCardProps extends Activity {}
export declare function ActivityCard(props: ActivityCardProps): React.ReactElement;

export interface ToolbarProps { title: React.ReactNode; subtitle?: React.ReactNode; children?: React.ReactNode; style?: React.CSSProperties }
export declare function Toolbar(props: ToolbarProps): React.ReactElement;

export interface ConnectStepsProps { steps?: string[] }
export declare function ConnectSteps(props: ConnectStepsProps): React.ReactElement;

declare global {
  interface Window {
    TmpNam: {
      Button: typeof Button; Icon: typeof Icon; Tag: typeof Tag; StatusDot: typeof StatusDot; Spinner: typeof Spinner;
      ProgressBar: typeof ProgressBar; Banner: typeof Banner; TextField: typeof TextField; Checkbox: typeof Checkbox;
      Radio: typeof Radio; PopupButton: typeof PopupButton; Menu: typeof Menu; SegmentedControl: typeof SegmentedControl;
      SizePicker: typeof SizePicker; GainControl: typeof GainControl; StageList: typeof StageList; Sheet: typeof Sheet;
      Sidebar: typeof Sidebar; UnitStatus: typeof UnitStatus; ActivityCard: typeof ActivityCard; Toolbar: typeof Toolbar;
      ConnectSteps: typeof ConnectSteps; utils: { gainDb(x: number): string };
    };
  }
}
