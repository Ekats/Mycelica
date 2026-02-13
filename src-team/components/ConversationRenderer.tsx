import ReactMarkdown from "react-markdown";

interface Message {
  role: "human" | "assistant";
  content: string;
}

interface ConversationRendererProps {
  content: string;
}

/**
 * Auto-format plain text to markdown:
 * - Wrap file paths in backticks
 * - Wrap shell commands in code blocks
 * - Format numbered lists
 */
function autoFormatMarkdown(text: string): string {
  let result = text;

  // Skip if already has fenced code blocks
  if (result.includes("```")) {
    return result;
  }

  // Detect multi-line command blocks and wrap them in code fences
  const commandPattern =
    /^((?:sudo |)\s*(?:mount|umount|blkid|grub-install|update-grub|manjaro-chroot|chroot|fdisk|lsblk|df|reboot|exit|apt|pacman|yum|dnf|systemctl|journalctl)[^\n]*(?:\n(?:sudo |)\s*(?:mount|umount|blkid|grub-install|update-grub|manjaro-chroot|chroot|fdisk|lsblk|df|reboot|exit|apt|pacman|yum|dnf|systemctl|journalctl)[^\n]*)*)$/gm;
  result = result.replace(commandPattern, "```bash\n$1\n```");

  // Single-line commands not already in backticks
  const singleCommands =
    /(?<!`)((?:sudo\s+)?(?:mount|umount|blkid|grub-install|update-grub|manjaro-chroot|chroot|fdisk|lsblk|reboot|exit)\s+[^\n`]+)/g;
  result = result.replace(singleCommands, "`$1`");

  // File paths: /path/to/file, /dev/xxx - but not already in backticks
  result = result.replace(
    /(?<!`)(\/(dev|mnt|boot|etc|usr|home|var|tmp)\/[\w./\-_]+)/g,
    "`$1`",
  );

  // Flags like --target=xxx
  result = result.replace(/(?<!`)(--[\w-]+=[\w\-/]+)/g, "`$1`");

  // Format "Step N:" or numbered steps as headers
  result = result.replace(/^(\d+\.\s+)([A-Z][^:\n]+:)/gm, "\n**$2**\n");

  return result;
}

function parseConversation(content: string): Message[] {
  const parts = content.split(/(?=Human:|H:|Assistant:|A:)/);
  const messages: Message[] = [];

  for (const part of parts) {
    const trimmed = part.trim();
    if (!trimmed) continue;

    let role: "human" | "assistant";
    let messageContent: string;

    if (trimmed.startsWith("Human:")) {
      role = "human";
      messageContent = trimmed.slice(6).trim();
    } else if (trimmed.startsWith("H:")) {
      role = "human";
      messageContent = trimmed.slice(2).trim();
    } else if (trimmed.startsWith("Assistant:")) {
      role = "assistant";
      messageContent = trimmed.slice(10).trim();
    } else if (trimmed.startsWith("A:")) {
      role = "assistant";
      messageContent = trimmed.slice(2).trim();
    } else {
      role = "human";
      messageContent = trimmed;
    }

    if (messageContent) {
      messages.push({ role, content: messageContent });
    }
  }

  return messages;
}

export function ConversationRenderer({ content }: ConversationRendererProps) {
  const messages = parseConversation(content);

  if (messages.length === 0) {
    return (
      <div className="text-gray-400 text-center py-4">No messages found</div>
    );
  }

  return (
    <article className="space-y-4">
      {messages.map((message, index) => (
        <section key={index}>
          <div
            className={`text-xs font-semibold mb-2 pb-1 border-b ${
              message.role === "human"
                ? "text-amber-400 border-amber-400/30"
                : "text-gray-400 border-gray-600"
            }`}
          >
            {message.role === "human" ? "You" : "Claude"}
          </div>
          <div
            className="prose prose-invert prose-sm max-w-none
            prose-headings:text-white prose-headings:font-semibold prose-headings:mt-3 prose-headings:mb-1
            prose-p:text-gray-300 prose-p:leading-relaxed prose-p:my-2
            prose-a:text-amber-400 prose-a:no-underline hover:prose-a:underline
            prose-strong:text-white
            prose-em:text-gray-200
            prose-code:text-amber-300 prose-code:bg-gray-950 prose-code:px-1 prose-code:py-0.5 prose-code:rounded prose-code:text-xs prose-code:font-mono prose-code:border prose-code:border-gray-700
            prose-pre:bg-gray-950 prose-pre:border prose-pre:border-gray-600 prose-pre:rounded-lg prose-pre:my-3 prose-pre:p-3 prose-pre:overflow-x-auto
            [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-green-400 [&_pre_code]:border-0
            prose-blockquote:border-l-2 prose-blockquote:border-amber-500/50 prose-blockquote:bg-gray-800/30 prose-blockquote:py-1 prose-blockquote:px-3 prose-blockquote:my-3 prose-blockquote:italic prose-blockquote:text-gray-400
            prose-ul:my-2 prose-ul:pl-4
            prose-ol:my-2 prose-ol:pl-4
            prose-li:text-gray-300 prose-li:my-0.5
            prose-hr:border-gray-700 prose-hr:my-4
          "
          >
            <ReactMarkdown>
              {autoFormatMarkdown(message.content)}
            </ReactMarkdown>
          </div>
        </section>
      ))}
    </article>
  );
}

export function isConversationContent(content: string): boolean {
  const hasHumanMarker = /(?:^|\n)(?:Human:|H:)/m.test(content);
  const hasAssistantMarker = /(?:^|\n)(?:Assistant:|A:)/m.test(content);
  return hasHumanMarker && hasAssistantMarker;
}
