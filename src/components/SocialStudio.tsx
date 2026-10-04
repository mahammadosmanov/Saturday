import {
	ArrowLeft,
	CalendarDays,
	Check,
	ChartNoAxesCombined,
	ChevronLeft,
	ChevronRight,
	FileText,
	Inbox,
	Plus,
	Search,
	Send,
	Trash2,
	UsersRound,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Modal } from "./Modal";

const platforms = [
	"YouTube",
	"Instagram",
	"Facebook",
	"X",
	"LinkedIn",
	"TikTok",
	"Threads",
	"Pinterest",
	"Reddit",
	"Bluesky",
	"Telegram",
	"Mastodon",
	"Snapchat",
	"WhatsApp Channels",
	"Google Business Profile",
	"Twitch",
	"Discord",
	"Tumblr",
] as const;
type Platform = (typeof platforms)[number];
type Post = {
	id: string;
	text: string;
	platforms: Platform[];
	plannedFor: string;
	createdAt: string;
};
type View =
	| "planner"
	| "posts"
	| "publish"
	| "channels"
	| "inbox"
	| "insights"
	| "compose";
const STORAGE_KEY = "jarvis.social-studio.v1";

function loadPosts(): {
	posts: Post[];
	warning: string;
	writable: boolean;
	raw: string | null;
} {
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		if (!raw) return { posts: [], warning: "", writable: true, raw };
		if (raw.length > 1_000_000) throw new Error("size");
		const value: unknown = JSON.parse(raw);
		if (
			!value ||
			typeof value !== "object" ||
			!("version" in value) ||
			value.version !== 1 ||
			!("posts" in value) ||
			!Array.isArray(value.posts)
		)
			throw new Error("schema");
		const posts = value.posts
			.slice(0, 100)
			.filter(
				(entry): entry is Post =>
					!!entry &&
					typeof entry === "object" &&
					typeof entry.id === "string" &&
					entry.id.length <= 100 &&
					typeof entry.text === "string" &&
					entry.text.length <= 5000 &&
					Array.isArray(entry.platforms) &&
					entry.platforms.length <= platforms.length &&
					entry.platforms.every((platform: unknown) =>
						platforms.includes(platform as Platform),
					) &&
					typeof entry.plannedFor === "string" &&
					entry.plannedFor.length <= 32 &&
					typeof entry.createdAt === "string" &&
					entry.createdAt.length <= 32,
			);
		return { posts, warning: "", writable: true, raw };
	} catch {
		return {
			posts: [],
			warning:
				"Saved planner data could not be read. It is preserved; saving is paused to protect it.",
			writable: false,
			raw: null,
		};
	}
}

function localInput(date: Date) {
	const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
	return local.toISOString().slice(0, 16);
}
function dayKey(date: Date) {
	return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
function weekStart(offset: number) {
	const date = new Date();
	date.setHours(0, 0, 0, 0);
	date.setDate(date.getDate() - ((date.getDay() + 6) % 7) + offset * 7);
	return date;
}
function displayDate(value: string) {
	const date = new Date(value);
	return Number.isNaN(date.getTime())
		? "Date not set"
		: date.toLocaleString([], {
				weekday: "short",
				month: "short",
				day: "numeric",
				hour: "numeric",
				minute: "2-digit",
			});
}

export function SocialStudio({
	open,
	onClose,
}: {
	open: boolean;
	onClose: () => void;
}) {
	const [initial] = useState(loadPosts);
	const [posts, setPosts] = useState(initial.posts);
	const [storageWarning, setStorageWarning] = useState(initial.warning);
	const [storageWritable, setStorageWritable] = useState(initial.writable);
	const lastStoredValue = useRef(initial.raw);
	const [view, setView] = useState<View>("planner");
	const [weekOffset, setWeekOffset] = useState(0);
	const [query, setQuery] = useState("");
	const [filter, setFilter] = useState<"all" | "draft" | "planned">("all");
	const [content, setContent] = useState("");
	const [targets, setTargets] = useState<Platform[]>([]);
	const [plannedFor, setPlannedFor] = useState("");
	const [formError, setFormError] = useState("");
	const [notice, setNotice] = useState("");
	const [editingId, setEditingId] = useState<string | null>(null);
	const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
	useEffect(() => {
		if (!storageWritable) return;
		try {
			const current = localStorage.getItem(STORAGE_KEY);
			if (current !== lastStoredValue.current) {
				setStorageWritable(false);
				setStorageWarning(
					"Planner data changed in another window. This session will not overwrite it; reopen Social Studio to load the latest copy.",
				);
				return;
			}
			const next = JSON.stringify({ version: 1, posts });
			localStorage.setItem(STORAGE_KEY, next);
			lastStoredValue.current = next;
		} catch {
			setStorageWritable(false);
			setStorageWarning(
				"Browser storage is unavailable. New posts are only in this session.",
			);
		}
	}, [posts, storageWritable]);
	useEffect(() => {
		const checkOtherWindow = (event: StorageEvent) => {
			if (event.key !== STORAGE_KEY) return;
			setStorageWritable(false);
			setStorageWarning(
				"Planner data changed in another window. This session will not overwrite it; reopen Social Studio to load the latest copy.",
			);
		};
		window.addEventListener("storage", checkOtherWindow);
		return () => window.removeEventListener("storage", checkOtherWindow);
	}, []);
	const newPost = () => {
		setContent("");
		setTargets([]);
		setPlannedFor("");
		setEditingId(null);
		setFormError("");
		setNotice("");
		setView("compose");
	};
	const savePost = (planned: boolean) => {
		if (!content.trim() || !targets.length || (planned && !plannedFor)) {
			setFormError(
				"Add post text, choose at least one channel, and set a date for planned posts.",
			);
			return;
		}
		const prior = posts.find((item) => item.id === editingId);
		const post: Post = {
			id: prior?.id ?? crypto.randomUUID(),
			text: content.trim(),
			platforms: targets,
			plannedFor: planned ? plannedFor : "",
			createdAt: prior?.createdAt ?? new Date().toISOString(),
		};
		setPosts((items) =>
			prior
				? items.map((item) => (item.id === prior.id ? post : item))
				: [post, ...items].slice(0, 100),
		);
		setNotice(
			!storageWritable
				? "Kept for this session only; browser storage is unavailable."
				: prior
					? "Post updated."
					: planned
						? "Added to your local planner."
						: "Draft saved on this device.",
		);
		setView(planned ? "planner" : "posts");
	};
	const removePost = (id: string) =>
		setPosts((items) => items.filter((post) => post.id !== id));
	const editPost = (post: Post) => {
		setEditingId(post.id);
		setContent(post.text);
		setTargets(post.platforms);
		setPlannedFor(post.plannedFor);
		setFormError("");
		setView("compose");
	};
	const cancelCompose = () => {
		const original = posts.find((post) => post.id === editingId);
		const changed = original
			? content.trim() !== original.text ||
				JSON.stringify(targets) !== JSON.stringify(original.platforms) ||
				plannedFor !== original.plannedFor
			: !!content.trim() || targets.length > 0 || !!plannedFor;
		if (!changed || window.confirm("Discard these unsaved changes?")) {
			setView("posts");
			setEditingId(null);
		}
	};
	const start = weekStart(weekOffset);
	const days = Array.from({ length: 7 }, (_, index) => {
		const date = new Date(start);
		date.setDate(start.getDate() + index);
		return date;
	});
	const visiblePosts = posts.filter((post) => {
		const matchesText =
			!query ||
			post.text.toLowerCase().includes(query.toLowerCase()) ||
			post.platforms.some((platform) =>
				platform.toLowerCase().includes(query.toLowerCase()),
			);
		const matchesFilter =
			view !== "posts" ||
			filter === "all" ||
			(filter === "draft" ? !post.plannedFor : !!post.plannedFor);
		return matchesText && matchesFilter;
	});
	const plannedPosts = visiblePosts.filter((post) => post.plannedFor);
	const upcomingPosts = plannedPosts
		.filter((post) => new Date(post.plannedFor).getTime() >= Date.now())
		.sort((a, b) => a.plannedFor.localeCompare(b.plannedFor))
		.slice(0, 4);
	const weekLabel = `${start.toLocaleDateString([], { month: "short", day: "numeric" })} – ${days[6].toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" })}`;
	const title =
		view === "planner"
			? "Planner"
			: view === "posts"
				? "Posts"
				: view === "publish"
					? "Publishing"
					: view === "channels"
						? "Channels"
						: view === "inbox"
							? "Inbox"
							: view === "insights"
								? "Insights"
								: "New post";
	const nav = [
		["planner", CalendarDays, "Planner"],
		["posts", FileText, "Posts"],
		["publish", Send, "Publishing"],
		["channels", UsersRound, "Channels"],
		["inbox", Inbox, "Inbox"],
		["insights", ChartNoAxesCombined, "Insights"],
	] as const;
	return (
		<Modal
			open={open}
			onOpenChange={(next) => !next && onClose()}
			title="Social Studio"
			description="Plan content across channels. Posts are stored on this device; publishing is not connected."
			className="social-workspace-dialog"
		>
			<div className="social-workspace">
				<aside className="social-sidebar">
					<div className="social-sidebar-title">WORKSPACE</div>
					<nav aria-label="Social Studio sections">
						{nav.map(([id, Icon, label]) => (
							<button
								key={id}
								type="button"
								className={view === id ? "is-active" : ""}
								aria-current={view === id ? "page" : undefined}
								onClick={() => {
									setView(id);
									setNotice("");
								}}
							>
								<Icon size={16} />
								<span>{label}</span>
								{id === "posts" && <small>{posts.length}</small>}
							</button>
						))}
					</nav>
					<div className="social-sidebar-foot">
						<span className="social-local-dot" />
						Stored in this app on this device
					</div>
				</aside>
				<div className="social-main">
					<header className="social-toolbar">
						<div className="social-title-group">
							{view === "compose" && (
								<button
									className="social-back"
									type="button"
									aria-label="Back to posts"
									onClick={cancelCompose}
								>
									<ArrowLeft size={17} />
								</button>
							)}
							<h2>{title}</h2>
							{view === "planner" && (
								<span className="social-range">{weekLabel}</span>
							)}
						</div>
						<div className="social-toolbar-actions">
							{(view === "posts" || view === "planner") && (
								<label className="social-search">
									<Search size={15} />
									<input
										aria-label="Search posts"
										value={query}
										onChange={(event) => setQuery(event.target.value)}
										placeholder="Search posts"
									/>
								</label>
							)}
							<button
								className="button primary social-new-button"
								type="button"
								onClick={newPost}
							>
								<Plus size={16} /> New post
							</button>
						</div>
					</header>
					{storageWarning && (
						<div className="social-warning" role="status">
							{storageWarning}
						</div>
					)}
					{notice && (
						<div className="social-notice" role="status">
							{notice}
						</div>
					)}
					{view === "planner" && (
						<div className="social-planner-layout">
							<section
								className="social-calendar"
								aria-label="Weekly content planner"
							>
								<div className="social-calendar-head">
									<div>
										<h3>This week</h3>
										<p>Planned posts by day and channel</p>
									</div>
									<div className="social-week-controls">
										<button
											type="button"
											aria-label="Previous week"
											onClick={() => setWeekOffset((n) => n - 1)}
										>
											<ChevronLeft size={17} />
										</button>
										<button
											type="button"
											aria-label="Next week"
											onClick={() => setWeekOffset((n) => n + 1)}
										>
											<ChevronRight size={17} />
										</button>
										<button type="button" onClick={() => setWeekOffset(0)}>
											Today
										</button>
									</div>
								</div>
								<div className="social-week-list">
									{days.map((date) => {
										const items = plannedPosts.filter(
											(post) =>
												post.plannedFor &&
												dayKey(new Date(post.plannedFor)) === dayKey(date),
										);
										const today = dayKey(date) === dayKey(new Date());
										return (
											<div
												className={`social-day ${today ? "is-today" : ""}`}
												key={dayKey(date)}
											>
												<div className="social-day-date">
													<span>
														{date.toLocaleDateString([], { weekday: "short" })}
													</span>
													<strong>{date.getDate()}</strong>
												</div>
												<div className="social-day-items">
													{items.map((post) => (
														<button
															className="social-event"
															key={post.id}
															aria-label={`Edit planned post: ${post.text}`}
															type="button"
															onClick={() => editPost(post)}
														>
															<time>
																{new Date(post.plannedFor).toLocaleTimeString(
																	[],
																	{ hour: "numeric", minute: "2-digit" },
																)}
															</time>
															<span className="social-event-copy">
																{post.text}
															</span>
															<span className="social-event-platforms">
																{post.platforms.join(" · ")}
															</span>
														</button>
													))}
													<button
														className="social-day-empty"
														aria-label={`Add post on ${date.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" })}`}
														type="button"
														onClick={() => {
															newPost();
															setPlannedFor(
																localInput(
																	new Date(
																		date.getFullYear(),
																		date.getMonth(),
																		date.getDate(),
																		12,
																	),
																),
															);
														}}
													>
														Add a post <Plus size={13} />
													</button>
												</div>
											</div>
										);
									})}
								</div>
							</section>
							<aside className="social-upcoming">
								<div className="social-upcoming-head">
									<h3>Next up</h3>
									<button type="button" onClick={() => setView("posts")}>
										All posts
									</button>
								</div>
								{upcomingPosts.map((post) => (
									<article className="social-upcoming-item" key={post.id}>
										<time>{displayDate(post.plannedFor)}</time>
										<p>{post.text}</p>
										<span>{post.platforms.join(" · ")}</span>
									</article>
								))}
								{!upcomingPosts.length && (
									<div className="social-upcoming-empty">
										<CalendarDays size={20} />
										<p>
											{query
												? "No upcoming posts match your search."
												: "No upcoming posts."}
										</p>
										<button type="button" onClick={newPost}>
											Plan a post
										</button>
									</div>
								)}
							</aside>
						</div>
					)}
					{view === "posts" && (
						<section className="social-posts-view">
							<fieldset className="social-filter-row">
								<legend className="social-visually-hidden">Filter posts</legend>
								{(["all", "draft", "planned"] as const).map((value) => (
									<button
										type="button"
										key={value}
										aria-pressed={filter === value}
										onClick={() => setFilter(value)}
									>
										{value === "all"
											? "All posts"
											: value === "draft"
												? "Drafts"
												: "Planned"}
										<span>
											{value === "all"
												? posts.length
												: posts.filter((post) =>
														value === "draft"
															? !post.plannedFor
															: !!post.plannedFor,
													).length}
										</span>
									</button>
								))}
							</fieldset>
							{visiblePosts.length ? (
								<div className="social-post-list">
									{visiblePosts.map((post) => (
										<article className="social-post-row" key={post.id}>
											<div className="social-post-state">
												<span className={post.plannedFor ? "planned" : "draft"}>
													{post.plannedFor ? "Planned" : "Draft"}
												</span>
												<time>
													{post.plannedFor
														? displayDate(post.plannedFor)
														: `Created ${displayDate(post.createdAt)}`}
												</time>
											</div>
											<p>{post.text}</p>
											<div className="social-post-meta">
												<span>{post.platforms.join(" · ")}</span>
												<div className="social-post-actions">
													{confirmingDelete === post.id ? (
														<>
															<span>Delete this post?</span>
															<button
																type="button"
																onClick={() => setConfirmingDelete(null)}
															>
																Keep
															</button>
															<button
																type="button"
																onClick={() => {
																	removePost(post.id);
																	setConfirmingDelete(null);
																}}
															>
																Delete
															</button>
														</>
													) : (
														<>
															<button
																type="button"
																onClick={() => editPost(post)}
															>
																Edit
															</button>
															<button
																type="button"
																aria-label="Delete post"
																title="Delete post"
																onClick={() => setConfirmingDelete(post.id)}
															>
																<Trash2 size={15} />
															</button>
														</>
													)}
												</div>
											</div>
										</article>
									))}
								</div>
							) : (
								<div className="social-empty">
									<FileText size={24} />
									<h3>
										{query
											? "No posts match your search"
											: "Your post list is empty"}
									</h3>
									<p>
										{query
											? "Try another phrase or clear the search field."
											: "Create a draft or plan a post to start building your content calendar."}
									</p>
									{!query && (
										<button
											className="button primary"
											type="button"
											onClick={newPost}
										>
											<Plus size={15} /> Create your first post
										</button>
									)}
								</div>
							)}
						</section>
					)}
					{view === "publish" && (
						<section className="social-publish-view">
							<div className="social-view-intro">
								<h3>Publishing queue</h3>
								<p>
									Planned posts appear here with channel readiness. Publishing
									is unavailable until individual accounts can connect.
								</p>
							</div>
							{posts.filter((post) => post.plannedFor).length ? (
								<div className="social-post-list">
									{posts
										.filter((post) => post.plannedFor)
										.sort((a, b) => a.plannedFor.localeCompare(b.plannedFor))
										.map((post) => (
											<article className="social-post-row" key={post.id}>
												<div className="social-post-state">
													<span>Waiting for connection</span>
													<time>{displayDate(post.plannedFor)}</time>
												</div>
												<p>{post.text}</p>
												<div className="social-post-meta">
													<span>{post.platforms.join(" · ")}</span>
													<button
														type="button"
														disabled
														title="Connect the target accounts before publishing"
													>
														Publish now
													</button>
												</div>
											</article>
										))}
								</div>
							) : (
								<div className="social-empty">
									<Send size={24} />
									<h3>No planned posts</h3>
									<p>
										Posts you add to the planner will appear here with their
										channel readiness.
									</p>
									<button
										className="button primary"
										type="button"
										onClick={newPost}
									>
										<Plus size={15} /> Plan a post
									</button>
								</div>
							)}
						</section>
					)}
					{view === "channels" && (
						<section
							className="social-channel-view"
							// biome-ignore lint/a11y/noNoninteractiveTabindex: required to keyboard-scroll this overflow region in Safari
							tabIndex={0}
						>
							<div className="social-view-intro">
								<h3>Channels</h3>
								<p>
									Connect each profile separately when account linking is
									available.
								</p>
							</div>
							<section
								className="social-channel-list"
								aria-label="Connected channel accounts"
							>
								{platforms.map((platform) => (
									<div className="social-channel-row" key={platform}>
										<span className="social-platform-mark">
											{platform.slice(0, 2).toUpperCase()}
										</span>
										<strong>{platform}</strong>
										<span className="social-disconnected">Not connected</span>
										<button
											type="button"
											disabled
											title="Account linking is not configured in this build"
										>
											Connect account
										</button>
									</div>
								))}
							</section>
						</section>
					)}
					{(view === "inbox" || view === "insights") && (
						<section className="social-unavailable">
							<div className="social-unavailable-icon">
								{view === "inbox" ? (
									<Inbox size={22} />
								) : (
									<ChartNoAxesCombined size={22} />
								)}
							</div>
							<h3>
								{view === "inbox"
									? "No connected accounts"
									: "Insights need account data"}
							</h3>
							<p>
								{view === "inbox"
									? "Connect a supported account to manage comments and messages here."
									: "Once account connections are available, this space can show post reach, engagement, and channel comparisons."}
							</p>
							<button type="button" onClick={() => setView("channels")}>
								View channels
							</button>
						</section>
					)}
					{view === "compose" && (
						<section className="social-compose-view">
							<form onSubmit={(event) => event.preventDefault()}>
								<div className="social-compose-grid">
									<div className="social-compose-editor">
										<label htmlFor="post-copy">Post text</label>
										<textarea
											id="post-copy"
											value={content}
											maxLength={5000}
											onChange={(event) => {
												setContent(event.target.value);
												setFormError("");
											}}
											placeholder="Write your post…"
										/>
										<div className="social-editor-count">
											{content.length} / 5,000
										</div>
										<fieldset className="social-targets">
											<legend>Channels</legend>
											<p>
												Select one or more channels. Each channel can be
												connected separately later.
											</p>
											<div>
												{platforms.map((platform) => (
													<button
														type="button"
														key={platform}
														aria-pressed={targets.includes(platform)}
														onClick={() => {
															setTargets((items) =>
																items.includes(platform)
																	? items.filter((item) => item !== platform)
																	: [...items, platform],
															);
															setFormError("");
														}}
													>
														<span>
															{targets.includes(platform) && (
																<Check size={11} aria-hidden="true" />
															)}
														</span>
														{platform}
													</button>
												))}
											</div>
										</fieldset>
									</div>
									<aside className="social-compose-side">
										<div className="social-preview">
											<div className="social-preview-head">
												<span className="social-platform-mark">
													{targets[0]?.slice(0, 2).toUpperCase() || "CH"}
												</span>
												<div>
													<strong>{targets[0] || "Channel preview"}</strong>
													<small>
														{targets.length > 1
															? `+ ${targets.length - 1} more selected`
															: "Preview"}
													</small>
												</div>
											</div>
											<p>
												{content ||
													"Your post preview will appear here as you write."}
											</p>
											<span className="social-preview-foot">
												Preview only · platform formatting may differ
											</span>
										</div>
										<div className="social-plan-field">
											<label htmlFor="planned-for">Add to planner</label>
											<input
												id="planned-for"
												type="datetime-local"
												min={localInput(new Date())}
												value={plannedFor}
												onChange={(event) => setPlannedFor(event.target.value)}
											/>
											<p>
												Planning only. This will not publish to social
												platforms.
											</p>
										</div>
									</aside>
								</div>
								{formError && (
									<p className="social-form-error" role="alert">
										{formError}
									</p>
								)}
								<div className="social-compose-actions">
									<button type="button" onClick={cancelCompose}>
										Cancel
									</button>
									<div>
										<button
											type="button"
											className="button"
											onClick={() => savePost(false)}
										>
											{editingId ? "Update draft" : "Save draft"}
										</button>
										<button
											type="button"
											className="button primary"
											onClick={() => savePost(true)}
										>
											{editingId ? "Update plan" : "Add to planner"}
										</button>
									</div>
								</div>
							</form>
						</section>
					)}
				</div>
			</div>
		</Modal>
	);
}
