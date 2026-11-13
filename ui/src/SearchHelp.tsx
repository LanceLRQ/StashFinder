import Modal from "./Modal";

const examples = [
  ["report backup", "同时包含两个词。"],
  ['"annual report"', "匹配连续短语。"],
  ["<report|invoice> !draft", "匹配 report 或 invoice，并排除 draft。"],
  ["*.pdf", "使用通配符匹配文件名。"],
  ["path:projects", "搜索路径。离线分区使用卷内路径，可通过搜索栏限定分区。"],
  ['parent:"projects\\demo"', "只匹配指定父目录的直接子项。"],
  ["ancestor:projects", "匹配目录及其子目录。"],
  ["folder:", "仅匹配文件夹。"],
  ["ext:pdf;docx size:>=1MiB", "按扩展名和大小筛选。"],
  ["dm:today", "只匹配今天修改的记录。"],
  [
    'regex:"(?=report).*\\.pdf$"',
    "让此条件使用 PCRE2 正则。Ctrl+R 可切换整个输入框的正则模式。",
  ],
  ["dupe:name;size", "按名称和大小查找重复候选。结果基于索引元数据。"],
];

export default function SearchHelp({ onClose }: { onClose: () => void }) {
  return (
    <Modal title="搜索语法" className="search-filter-dialog" onClose={onClose}>
      <p>
        搜索默认包含已登记的在线和离线分区。可通过搜索栏选择分区，或在「搜索」菜单中启用「仅在线」「包含历史记录」。
      </p>
      <dl className="search-help-examples">
        {examples.map(([query, description]) => (
          <div key={query}>
            <dt>
              <code>{query}</code>
            </dt>
            <dd>{description}</dd>
          </div>
        ))}
      </dl>
    </Modal>
  );
}
