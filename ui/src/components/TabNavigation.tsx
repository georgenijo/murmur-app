export type TabType = 'current' | 'history';

interface TabNavigationProps {
  activeTab: TabType;
  onTabChange: (tab: TabType) => void;
  historyCount: number;
}

export function TabNavigation({ activeTab, onTabChange, historyCount }: TabNavigationProps) {
  return (
    <div className="flex-shrink-0 flex gap-1 bg-gray-100 dark:bg-gray-800 p-1 rounded-lg">
      <button
        onClick={() => onTabChange('current')}
        className={`flex-1 px-4 py-2 text-sm font-medium rounded-md transition-colors ${
          activeTab === 'current'
            ? 'bg-white dark:bg-gray-700 text-gray-900 dark:text-white shadow-sm'
            : 'text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'
        }`}
      >
        Current
      </button>
      <button
        onClick={() => onTabChange('history')}
        className={`flex-1 px-4 py-2 text-sm font-medium rounded-md transition-colors ${
          activeTab === 'history'
            ? 'bg-white dark:bg-gray-700 text-gray-900 dark:text-white shadow-sm'
            : 'text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'
        }`}
      >
        History
        {historyCount > 0 && (
          <span className="ml-1.5 px-1.5 py-0.5 text-xs bg-gray-200 dark:bg-gray-600 rounded-full">
            {historyCount}
          </span>
        )}
      </button>
    </div>
  );
}
