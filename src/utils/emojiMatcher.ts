import emojiData from 'unicode-emoji-json/data-by-group.json'
import emojilib from 'emojilib'

interface EmojiEntry {
  emoji: string
  name: string
  slug: string
  skin_tone_support: boolean
  unicode_version: string
  emoji_version: string
}

interface EmojiGroup {
  name: string
  slug: string
  emojis: EmojiEntry[]
}

// Build a searchable index from keywords to emojis
const emojiIndex: Map<string, string> = new Map()

// Process emojilib - this has the most extensive keyword mappings
Object.entries(emojilib).forEach(([emoji, keywords]) => {
  if (Array.isArray(keywords)) {
    keywords.forEach(keyword => {
      const key = keyword.toLowerCase().replace(/_/g, ' ')
      if (!emojiIndex.has(key)) {
        emojiIndex.set(key, emoji)
      }
      const singleWord = keyword.toLowerCase().replace(/_/g, '')
      if (!emojiIndex.has(singleWord) && singleWord.length > 2) {
        emojiIndex.set(singleWord, emoji)
      }
    })
  }
})

// Process unicode-emoji-json for additional coverage
;(emojiData as EmojiGroup[]).forEach(group => {
  group.emojis.forEach(entry => {
    const name = entry.name.toLowerCase()
    if (!emojiIndex.has(name)) {
      emojiIndex.set(name, entry.emoji)
    }
    const words = name.split(/[\s\-_]+/)
    words.forEach(word => {
      if (word.length > 2 && !emojiIndex.has(word)) {
        emojiIndex.set(word, entry.emoji)
      }
    })
  })
})

// Custom keyword mappings for tech/programming topics (override generic matches)
const customMappings: Record<string, string> = {
  // Programming Languages & Frameworks
  'python': '🐍',
  'javascript': '📜',
  'typescript': '📘',
  'react': '⚛️',
  'vue': '💚',
  'angular': '🅰️',
  'node': '💚',
  'nodejs': '💚',
  'java': '☕',
  'kotlin': '🎯',
  'swift': '🦅',
  'rust': '🦀',
  'go': '🐹',
  'golang': '🐹',
  'ruby': '💎',
  'php': '🐘',
  'csharp': '🎵',
  'c#': '🎵',
  'cpp': '⚙️',
  'c++': '⚙️',
  'tauri': '🦀',
  'svelte': '🔥',
  'nextjs': '▲',
  'next': '▲',
  'vite': '⚡',
  'webpack': '📦',

  // Tech & Development
  'code': '💻',
  'coding': '💻',
  'programming': '💻',
  'software': '💻',
  'developer': '👨‍💻',
  'api': '🔌',
  'rest': '🔌',
  'graphql': '◼️',
  'database': '🗄️',
  'sql': '🗄️',
  'nosql': '🗄️',
  'mongodb': '🍃',
  'postgres': '🐘',
  'postgresql': '🐘',
  'mysql': '🐬',
  'sqlite': '📦',
  'redis': '🔴',
  'server': '🖥️',
  'backend': '⚙️',
  'frontend': '🎨',
  'fullstack': '🥞',
  'devops': '🔄',
  'docker': '🐳',
  'kubernetes': '☸️',
  'k8s': '☸️',
  'aws': '☁️',
  'azure': '☁️',
  'gcp': '☁️',
  'cloud': '☁️',
  'linux': '🐧',
  'ubuntu': '🐧',
  'windows': '🪟',
  'macos': '🍎',
  'terminal': '💻',
  'bash': '💻',
  'shell': '🐚',
  'zsh': '🐚',
  'git': '📝',
  'github': '🐙',
  'gitlab': '🦊',
  'version': '📝',
  'deploy': '🚀',
  'deployment': '🚀',
  'cicd': '🔄',
  'pipeline': '🔄',
  'test': '🧪',
  'testing': '🧪',
  'unittest': '🧪',
  'debug': '🐛',
  'debugging': '🐛',
  'bug': '🐛',
  'error': '❌',
  'exception': '⚠️',
  'fix': '🔧',
  'refactor': '♻️',
  'optimize': '⚡',
  'performance': '⚡',
  'security': '🔒',
  'auth': '🔐',
  'authentication': '🔐',
  'authorization': '🔐',
  'encryption': '🔐',
  'password': '🔑',
  'token': '🎟️',
  'jwt': '🎟️',
  'oauth': '🔐',
  'async': '⏳',
  'await': '⏳',
  'promise': '🤝',
  'callback': '↩️',
  'hook': '🪝',
  'hooks': '🪝',
  'component': '🧩',
  'state': '📊',
  'redux': '🔮',
  'zustand': '🐻',
  'context': '📋',

  // AI & Machine Learning
  'ai': '🤖',
  'artificial': '🤖',
  'intelligence': '🤖',
  'machine': '🤖',
  'ml': '🧠',
  'deep': '🧠',
  'learning': '🧠',
  'neural': '🧠',
  'network': '🕸️',
  'model': '🎯',
  'training': '🏋️',
  'dataset': '📊',
  'tensor': '🔢',
  'pytorch': '🔥',
  'tensorflow': '🔶',
  'gpt': '🤖',
  'chatgpt': '🤖',
  'claude': '🧠',
  'anthropic': '🧠',
  'llm': '🤖',
  'nlp': '💬',
  'prompt': '💬',
  'embedding': '📍',
  'embeddings': '📍',
  'vector': '📍',
  'rag': '📚',
  'agent': '🤖',
  'langchain': '🔗',
  'openai': '🤖',
  'huggingface': '🤗',
  'transformer': '🔄',
  'attention': '👁️',
  'finetune': '🎛️',
  'finetuning': '🎛️',

  // Data & Analytics
  'data': '📊',
  'analytics': '📈',
  'visualization': '📊',
  'chart': '📈',
  'graph': '📊',
  'dashboard': '📊',
  'report': '📋',
  'metric': '📏',
  'kpi': '📏',
  'statistics': '📉',
  'pandas': '🐼',
  'numpy': '🔢',
  'jupyter': '📓',
  'notebook': '📓',
  'clustering': '🔮',
  'cluster': '🔮',

  // Web & Internet
  'web': '🌐',
  'website': '🌐',
  'http': '🌐',
  'https': '🔒',
  'url': '🔗',
  'link': '🔗',
  'browser': '🌐',
  'chrome': '🌐',
  'firefox': '🦊',
  'safari': '🧭',
  'html': '📄',
  'css': '🎨',
  'sass': '🎨',
  'tailwind': '🌊',
  'bootstrap': '🅱️',
  'responsive': '📱',
  'mobile': '📱',
  'app': '📱',
  'ios': '🍎',
  'android': '🤖',
  'pwa': '📱',
  'spa': '⚡',
  'ssr': '🖥️',
  'seo': '🔍',

  // Writing & Content
  'write': '✍️',
  'writing': '✍️',
  'blog': '📝',
  'article': '📰',
  'post': '📝',
  'content': '📝',
  'copy': '📝',
  'copywriting': '✍️',
  'story': '📖',
  'book': '📚',
  'novel': '📖',
  'fiction': '📖',
  'essay': '📄',
  'poem': '🎭',
  'poetry': '🎭',
  'script': '🎬',
  'screenplay': '🎬',
  'documentation': '📚',
  'docs': '📚',
  'readme': '📖',
  'tutorial': '👨‍🏫',
  'guide': '📖',
  'manual': '📖',

  // Business & Work
  'business': '💼',
  'startup': '🚀',
  'entrepreneur': '💼',
  'company': '🏢',
  'corporate': '🏢',
  'office': '🏢',
  'work': '👔',
  'job': '💼',
  'career': '📈',
  'resume': '📄',
  'cv': '📄',
  'interview': '🎤',
  'hiring': '🤝',
  'meeting': '🤝',
  'presentation': '📊',
  'slide': '📊',
  'pitch': '🎯',
  'strategy': '♟️',
  'plan': '📅',
  'planning': '📅',
  'project': '📋',
  'task': '✅',
  'todo': '✅',
  'deadline': '⏰',
  'schedule': '📅',
  'calendar': '📅',
  'email': '📧',
  'mail': '📧',
  'message': '💬',
  'communication': '💬',
  'slack': '💬',
  'teams': '💬',
  'zoom': '📹',
  'video': '📹',
  'call': '📞',
  'conference': '🎤',

  // Finance & Money
  'money': '💰',
  'finance': '💵',
  'financial': '💵',
  'investment': '📈',
  'invest': '📈',
  'stock': '📈',
  'crypto': '₿',
  'bitcoin': '₿',
  'ethereum': '⟠',
  'blockchain': '🔗',
  'bank': '🏦',
  'banking': '🏦',
  'payment': '💳',
  'transaction': '💳',
  'budget': '💰',
  'tax': '🧾',
  'accounting': '🧮',
  'revenue': '💵',
  'profit': '💵',
  'sales': '💵',
  'pricing': '🏷️',

  // Science & Research
  'science': '🔬',
  'scientific': '🔬',
  'research': '🔍',
  'study': '📖',
  'experiment': '🧫',
  'hypothesis': '💡',
  'theory': '📐',
  'physics': '⚛️',
  'chemistry': '🧪',
  'biology': '🧬',
  'genetics': '🧬',
  'dna': '🧬',
  'medicine': '💊',
  'medical': '🏥',
  'health': '❤️',
  'healthcare': '🏥',
  'doctor': '👨‍⚕️',
  'patient': '🏥',
  'math': '🔢',
  'mathematics': '🔢',
  'calculus': '📐',
  'algebra': '🔢',
  'geometry': '📐',
  'equation': '🔢',
  'formula': '🔢',
  'algorithm': '📐',

  // Education
  'education': '🎓',
  'school': '🏫',
  'university': '🎓',
  'college': '🎓',
  'student': '👨‍🎓',
  'teacher': '👨‍🏫',
  'professor': '👨‍🏫',
  'course': '🎓',
  'class': '🎓',
  'lesson': '📖',
  'homework': '📝',
  'assignment': '📝',
  'exam': '📝',
  'quiz': '❓',
  'learn': '📚',
  'teach': '👨‍🏫',
  'explain': '💡',
  'understand': '🤔',
  'question': '❓',
  'answer': '✅',
  'help': '🆘',

  // Design & Creative
  'design': '🎨',
  'designer': '🎨',
  'ui': '🖼️',
  'ux': '👤',
  'interface': '🖼️',
  'wireframe': '📐',
  'mockup': '🖼️',
  'prototype': '🔧',
  'figma': '🎨',
  'sketch': '✏️',
  'photoshop': '🎨',
  'illustrator': '🎨',
  'graphic': '🎨',
  'logo': '🏷️',
  'brand': '🏷️',
  'branding': '🏷️',
  'color': '🌈',
  'font': '🔤',
  'typography': '🔤',
  'layout': '📐',
  'animation': '🎬',
  'motion': '🎬',
  '3d': '🎲',
  'render': '🖼️',
  'd3': '📊',

  // Media & Entertainment
  'music': '🎵',
  'song': '🎵',
  'audio': '🔊',
  'sound': '🔊',
  'podcast': '🎙️',
  'radio': '📻',
  'movie': '🎬',
  'film': '🎬',
  'cinema': '🎬',
  'tv': '📺',
  'television': '📺',
  'show': '📺',
  'series': '📺',
  'netflix': '📺',
  'youtube': '▶️',
  'stream': '📺',
  'streaming': '📺',
  'game': '🎮',
  'gaming': '🎮',
  'gamer': '🎮',
  'esports': '🎮',
  'player': '🎮',
  'art': '🎨',
  'artist': '🎨',
  'paint': '🎨',
  'draw': '✏️',
  'drawing': '✏️',
  'photo': '📷',
  'photography': '📷',
  'camera': '📷',
  'image': '🖼️',
  'picture': '🖼️',

  // Communication & Language
  'language': '🗣️',
  'translate': '🌍',
  'translation': '🌍',
  'english': '🇬🇧',
  'spanish': '🇪🇸',
  'french': '🇫🇷',
  'german': '🇩🇪',
  'chinese': '🇨🇳',
  'japanese': '🇯🇵',
  'korean': '🇰🇷',
  'portuguese': '🇧🇷',
  'italian': '🇮🇹',
  'russian': '🇷🇺',
  'arabic': '🇸🇦',
  'hindi': '🇮🇳',
  'grammar': '📝',
  'vocabulary': '📚',
  'speak': '🗣️',
  'speech': '🗣️',
  'voice': '🗣️',
  'conversation': '💬',
  'chat': '💭',
  'discuss': '💬',
  'debate': '💬',

  // Life & Personal
  'life': '🌱',
  'lifestyle': '🌱',
  'personal': '👤',
  'self': '👤',
  'growth': '🌱',
  'improve': '📈',
  'habit': '🔄',
  'routine': '🔄',
  'morning': '🌅',
  'evening': '🌆',
  'night': '🌙',
  'sleep': '😴',
  'dream': '💭',
  'goal': '🎯',
  'motivation': '💪',
  'inspire': '✨',
  'success': '🏆',
  'achieve': '🏆',
  'failure': '📉',
  'mindset': '🧠',
  'mental': '🧠',
  'psychology': '🧠',
  'emotion': '❤️',
  'feeling': '❤️',
  'happy': '😊',
  'sad': '😢',
  'stress': '😰',
  'anxiety': '😰',
  'relax': '😌',
  'meditation': '🧘',
  'mindfulness': '🧘',
  'yoga': '🧘',
  'fitness': '💪',
  'exercise': '🏃',
  'workout': '💪',
  'gym': '🏋️',
  'sport': '⚽',
  'run': '🏃',
  'running': '🏃',
  'walk': '🚶',
  'hiking': '🥾',
  'outdoor': '🏕️',
  'nature': '🌿',
  'environment': '🌍',
  'climate': '🌡️',
  'weather': '⛅',

  // Food & Cooking
  'food': '🍽️',
  'eat': '🍽️',
  'cook': '👨‍🍳',
  'cooking': '👨‍🍳',
  'recipe': '👨‍🍳',
  'meal': '🍽️',
  'breakfast': '🍳',
  'lunch': '🥪',
  'dinner': '🍽️',
  'restaurant': '🍽️',
  'kitchen': '👨‍🍳',
  'ingredient': '🥬',
  'nutrition': '🥗',
  'diet': '🥗',
  'vegetarian': '🥬',
  'vegan': '🥬',
  'coffee': '☕',
  'tea': '🍵',
  'drink': '🥤',
  'wine': '🍷',
  'beer': '🍺',

  // Travel & Places
  'travel': '✈️',
  'trip': '🧳',
  'vacation': '🏖️',
  'holiday': '🎄',
  'flight': '✈️',
  'airport': '✈️',
  'hotel': '🏨',
  'booking': '📅',
  'destination': '📍',
  'country': '🌍',
  'city': '🏙️',
  'map': '🗺️',
  'location': '📍',
  'adventure': '🏔️',
  'explore': '🔍',
  'tour': '🧳',
  'tourist': '📷',
  'beach': '🏖️',
  'mountain': '🏔️',
  'forest': '🌲',
  'ocean': '🌊',
  'island': '🏝️',

  // Home & Family
  'home': '🏠',
  'house': '🏠',
  'apartment': '🏢',
  'room': '🚪',
  'furniture': '🪑',
  'decor': '🖼️',
  'garden': '🌻',
  'family': '👨‍👩‍👧‍👦',
  'parent': '👨‍👩‍👧',
  'child': '👶',
  'baby': '👶',
  'kid': '🧒',
  'pet': '🐾',
  'dog': '🐕',
  'cat': '🐱',
  'relationship': '❤️',
  'love': '❤️',
  'friend': '🤝',
  'friendship': '🤝',
  'social': '👥',

  // Shopping & E-commerce
  'shop': '🛒',
  'shopping': '🛒',
  'buy': '🛒',
  'sell': '💵',
  'store': '🏪',
  'ecommerce': '🛒',
  'product': '📦',
  'order': '📦',
  'delivery': '🚚',
  'shipping': '🚚',
  'cart': '🛒',
  'checkout': '💳',
  'discount': '🏷️',
  'sale': '🏷️',
  'deal': '🤝',
  'offer': '🎁',
  'coupon': '🎟️',
  'customer': '👤',
  'review': '⭐',
  'rating': '⭐',

  // Legal & Contracts
  'legal': '⚖️',
  'law': '⚖️',
  'lawyer': '⚖️',
  'attorney': '⚖️',
  'contract': '📜',
  'agreement': '🤝',
  'terms': '📜',
  'policy': '📜',
  'privacy': '🔒',
  'copyright': '©️',
  'trademark': '™️',
  'patent': '📜',
  'license': '📜',
  'compliance': '✅',
  'regulation': '📜',

  // Misc & General
  'idea': '💡',
  'concept': '💡',
  'think': '🤔',
  'thinking': '🤔',
  'brainstorm': '🧠',
  'problem': '🔧',
  'solution': '✨',
  'solve': '✨',
  'create': '✨',
  'build': '🏗️',
  'make': '🛠️',
  'develop': '🔧',
  'generate': '⚡',
  'automate': '🤖',
  'automation': '🤖',
  'workflow': '🔄',
  'process': '⚙️',
  'system': '⚙️',
  'tool': '🔧',
  'utility': '🔧',
  'feature': '✨',
  'update': '🔄',
  'upgrade': '⬆️',
  'release': '🚀',
  'launch': '🚀',
  'announce': '📢',
  'news': '📰',
  'trend': '📈',
  'popular': '🔥',
  'viral': '🔥',
  'share': '🔗',
  'save': '💾',
  'download': '⬇️',
  'upload': '⬆️',
  'file': '📄',
  'folder': '📁',
  'document': '📄',
  'pdf': '📄',
  'excel': '📊',
  'word': '📝',
  'spreadsheet': '📊',
  'template': '📋',
  'example': '📌',
  'sample': '📌',
  'demo': '🎯',
  'preview': '👁️',
  'draft': '📝',
  'edit': '✏️',
  'delete': '🗑️',
  'remove': '🗑️',
  'add': '➕',
  'new': '🆕',
  'old': '📜',
  'archive': '📦',
  'backup': '💾',
  'restore': '🔄',
  'sync': '🔄',
  'connect': '🔗',
  'disconnect': '🔌',
  'online': '🌐',
  'offline': '📴',
  'wifi': '📶',
  'internet': '🌐',
  'setting': '⚙️',
  'config': '⚙️',
  'configuration': '⚙️',
  'option': '☑️',
  'preference': '⚙️',
  'customize': '🎨',
  'theme': '🎨',
  'dark': '🌙',
  'light': '☀️',
  'mode': '🔄',
  'toggle': '🔄',
  'switch': '🔄',
  'enable': '✅',
  'disable': '❌',
  'start': '▶️',
  'stop': '⏹️',
  'pause': '⏸️',
  'play': '▶️',
  'fast': '⚡',
  'slow': '🐢',
  'quick': '⚡',
  'instant': '⚡',
  'wait': '⏳',
  'loading': '⏳',
  'progress': '📊',
  'complete': '✅',
  'done': '✅',
  'finish': '🏁',
  'fail': '❌',
  'warning': '⚠️',
  'info': 'ℹ️',
  'tip': '💡',
  'note': '📝',
  'important': '❗',
  'urgent': '🚨',
  'priority': '🔝',
  'favorite': '⭐',
  'bookmark': '🔖',
  'pin': '📌',
  'tag': '🏷️',
  'label': '🏷️',
  'category': '📂',
  'group': '👥',
  'list': '📋',
  'table': '📊',
  'sort': '🔄',
  'filter': '🔍',
  'search': '🔍',
  'find': '🔍',
  'lookup': '🔍',
  'query': '🔍',
  'result': '📋',
  'output': '📤',
  'input': '📥',
  'form': '📝',
  'random': '🎲',
  'unique': '✨',
  'special': '⭐',
  'mycelica': '🍄',
  'mycelium': '🍄',
  'knowledge': '🧠',
  'memory': '🧠',
  'remember': '🧠',
  'recall': '🧠',
  'brain': '🧠',
  'neuron': '🧠',
  'synapse': '🔗',
}

// Add custom mappings to the index (these take priority)
Object.entries(customMappings).forEach(([keyword, emoji]) => {
  emojiIndex.set(keyword.toLowerCase(), emoji)
})

// Learned mappings from AI (loaded at runtime)
let learnedMappings: Map<string, string> = new Map()

/**
 * Initialize learned mappings from stored data
 */
export function initLearnedMappings(mappings: Record<string, string>) {
  learnedMappings = new Map(Object.entries(mappings))
}

/**
 * Add a learned mapping
 */
export function addLearnedMapping(keyword: string, emoji: string) {
  learnedMappings.set(keyword.toLowerCase(), emoji)
}

/**
 * Get all learned mappings
 */
export function getLearnedMappings(): Record<string, string> {
  return Object.fromEntries(learnedMappings)
}

/**
 * Result of emoji matching - includes whether AI should be consulted
 */
export interface EmojiMatchResult {
  emoji: string
  matched: boolean  // true if found in mappings, false if using default
  matchedKeyword?: string  // the keyword that matched
}

/**
 * Get the best matching emoji for a node based on its title, tags, and content
 * Returns both the emoji and whether it was a real match or default
 */
export function matchEmoji(
  title: string,
  tags?: string[] | string,
  content?: string
): EmojiMatchResult {
  // Handle tags as either array or JSON string
  let tagsArray: string[] = [];
  if (tags) {
    if (Array.isArray(tags)) {
      tagsArray = tags;
    } else if (typeof tags === 'string') {
      try {
        const parsed = JSON.parse(tags);
        tagsArray = Array.isArray(parsed) ? parsed : [];
      } catch {
        tagsArray = [];
      }
    }
  }
  const tagsText = tagsArray.join(' ')
  const searchText = (title + ' ' + tagsText).toLowerCase()

  // Split into words
  const words = searchText.split(/[\s\-_,.:;!?'"()\[\]{}]+/).filter(w => w.length > 2)

  // First check learned mappings (highest priority - user/AI corrections)
  for (const word of words) {
    if (learnedMappings.has(word)) {
      return { emoji: learnedMappings.get(word)!, matched: true, matchedKeyword: word }
    }
  }

  // Then check custom mappings (tech keywords)
  for (const word of words) {
    if (customMappings[word]) {
      return { emoji: customMappings[word], matched: true, matchedKeyword: word }
    }
  }

  // Then check the full emoji index
  for (const word of words) {
    const emoji = emojiIndex.get(word)
    if (emoji) {
      return { emoji, matched: true, matchedKeyword: word }
    }
  }

  // Try partial matches
  for (const word of words) {
    for (const [keyword, emoji] of Object.entries(customMappings)) {
      if (word.includes(keyword) || keyword.includes(word)) {
        return { emoji, matched: true, matchedKeyword: keyword }
      }
    }
  }

  // If we have content, try matching on that too
  if (content) {
    const contentWords = content.toLowerCase()
      .split(/[\s\-_,.:;!?'"()\[\]{}]+/)
      .filter(w => w.length > 3)
      .slice(0, 50) // Only check first 50 words for performance

    for (const word of contentWords) {
      if (customMappings[word]) {
        return { emoji: customMappings[word], matched: true, matchedKeyword: word }
      }
    }
  }

  // No match found - return default
  return { emoji: '💭', matched: false }
}

// Content type fallback emojis
const contentTypeEmoji: Record<string, string> = {
  idea: '💭',
  investigation: '🔍',
  code: '📝',
  debug: '🐛',
  paste: '📋',
  trivial: '💨',
}

/**
 * Get emoji for a node, using stored emoji if available
 */
export function getEmojiForNode(node: {
  title?: string
  aiTitle?: string
  tags?: string[] | string
  content?: string
  emoji?: string
  contentType?: string
}): string {
  // If node has a stored emoji, use it
  if (node.emoji) {
    return node.emoji
  }

  // Try to match from tags using the existing emojiIndex (thousands of mappings)
  if (node.tags) {
    const tagList = Array.isArray(node.tags)
      ? node.tags
      : node.tags.split(',').map(t => t.trim().toLowerCase())

    for (const tag of tagList) {
      const normalized = tag.toLowerCase().trim()
      // Check customMappings first (tech-specific), then emojiIndex
      if (customMappings[normalized]) {
        return customMappings[normalized]
      }
      if (emojiIndex.has(normalized)) {
        return emojiIndex.get(normalized)!
      }
    }
  }

  // Fall back to content_type emoji
  if (node.contentType && contentTypeEmoji[node.contentType]) {
    return contentTypeEmoji[node.contentType]
  }

  // Finally, compute from title
  const title = node.aiTitle || node.title || ''
  const result = matchEmoji(title, node.tags, node.content)
  return result.emoji
}

/**
 * Check if a title needs AI emoji suggestion
 */
export function needsAiEmoji(title: string, tags?: string[] | string): boolean {
  const result = matchEmoji(title, tags)
  return !result.matched
}

/**
 * Helper to normalize tags to array
 */
function normalizeTags(tags?: string[] | string): string[] {
  if (!tags) return [];
  if (Array.isArray(tags)) return tags;
  if (typeof tags === 'string') {
    try {
      const parsed = JSON.parse(tags);
      return Array.isArray(parsed) ? parsed : [];
    } catch {
      return [];
    }
  }
  return [];
}

/**
 * Get keywords that didn't match (for AI learning)
 */
export function getUnmatchedKeywords(title: string, tags?: string[] | string): string[] {
  const tagsArray = normalizeTags(tags);
  const tagsText = tagsArray.join(' ')
  const searchText = (title + ' ' + tagsText).toLowerCase()
  const words = searchText.split(/[\s\-_,.:;!?'"()\[\]{}]+/).filter(w => w.length > 2)

  return words.filter(word => {
    return !learnedMappings.has(word) &&
           !customMappings[word] &&
           !emojiIndex.has(word)
  })
}
