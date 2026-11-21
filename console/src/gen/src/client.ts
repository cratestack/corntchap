import { CratestackRuntime, type CratestackClientOptions, type CratestackResponseEnvelope } from "./runtime.js";
import { reviveWireFields, revivePagedWireFields, reviveWireScalar } from "./models.js";
import type {
  TaskItemView,
  TaskCommentView,
  TaskLinkView,
  TaskView,
  TaskDetail,
  LeaseView,
  ClaimResult,
  LeaseRenewal,
  MarkdownDoc,
  OkResult,
  ProjectSummary,
  TeamRole,
  WhoamiView,
  ChangePayload,
  ChangeView,
  ChangeFeed,
  WaitResult,
  Principal,
  UpdatePrincipalInput,
  PrincipalWhere,
  PrincipalOrderByClause,
  PrincipalFindMany,
  Team,
  CreateTeamInput,
  UpdateTeamInput,
  TeamWhere,
  TeamOrderByClause,
  TeamFindMany,
  TeamMember,
  CreateTeamMemberInput,
  UpdateTeamMemberInput,
  TeamMemberWhere,
  TeamMemberOrderByClause,
  TeamMemberFindMany,
  Project,
  CreateProjectInput,
  UpdateProjectInput,
  ProjectWhere,
  ProjectOrderByClause,
  ProjectFindMany,
  Task,
  UpdateTaskInput,
  TaskWhere,
  TaskOrderByClause,
  TaskFindMany,
  Item,
  UpdateItemInput,
  ItemWhere,
  ItemOrderByClause,
  ItemFindMany,
  Comment,
  UpdateCommentInput,
  CommentWhere,
  CommentOrderByClause,
  CommentFindMany,
  TaskDep,
  UpdateTaskDepInput,
  TaskDepWhere,
  TaskDepOrderByClause,
  TaskDepFindMany,
  WakeTarget,
  UpdateWakeTargetInput,
  WakeTargetWhere,
  WakeTargetOrderByClause,
  WakeTargetFindMany,
  Event,
  UpdateEventInput,
  EventWhere,
  EventOrderByClause,
  EventFindMany,
  WhoamiArgs,
  ProjectOverviewArgs,
  ProjectTasksArgs,
  TaskDetailArgs,
  GetBoardArgs,
  GetTaskMarkdownArgs,
  TaskCreateArgs,
  TaskEditArgs,
  ClaimNextArgs,
  ClaimTaskArgs,
  HeartbeatArgs,
  CheckItemArgs,
  CommentArgs,
  CompleteArgs,
  ReleaseArgs,
  BlockArgs,
  ChangesSinceArgs,
  WaitForWorkArgs,
  UnblockTaskArgs,
  CancelTaskArgs,
  PrincipalSortField,
  TeamSortField,
  TeamMemberSortField,
  ProjectSortField,
  TaskSortField,
  ItemSortField,
  CommentSortField,
  TaskDepSortField,
  WakeTargetSortField,
  EventSortField,
  Page,
  CursorPage,
} from "./models.js";
import {
  toSearchQuery,
  withIfMatchHeader,
  type CratestackQueryRequestConfig,
  type CratestackRequestConfig,
  type CratestackWriteRequestConfig,
} from "./queries.js";

export class CorntchapApiClient {
  readonly runtime: CratestackRuntime;
  readonly procedures: ProceduresApi;
  readonly principals: PrincipalApi;
  readonly teams: TeamApi;
  readonly teamMembers: TeamMemberApi;
  readonly projects: ProjectApi;
  readonly tasks: TaskApi;
  readonly items: ItemApi;
  readonly comments: CommentApi;
  readonly taskDeps: TaskDepApi;
  readonly wakeTargets: WakeTargetApi;
  readonly events: EventApi;

  constructor(originOrRuntime: string | CratestackRuntime, options: CratestackClientOptions = {}) {
    this.runtime = typeof originOrRuntime === "string"
      ? new CratestackRuntime(originOrRuntime, options)
      : originOrRuntime;
    this.procedures = new ProceduresApi(this.runtime);
    this.principals = new PrincipalApi(this.runtime);
    this.teams = new TeamApi(this.runtime);
    this.teamMembers = new TeamMemberApi(this.runtime);
    this.projects = new ProjectApi(this.runtime);
    this.tasks = new TaskApi(this.runtime);
    this.items = new ItemApi(this.runtime);
    this.comments = new CommentApi(this.runtime);
    this.taskDeps = new TaskDepApi(this.runtime);
    this.wakeTargets = new WakeTargetApi(this.runtime);
    this.events = new EventApi(this.runtime);
  }
}

export class PrincipalApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Principal[]> {
    return this.runtime.get<unknown>("/principals", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Principal') as Principal[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Principal> {
    return this.runtime.get<unknown>(`/principals/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Principal') as Principal);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Principal>> {
    return this.runtime.getWithResponse<unknown>(`/principals/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Principal') as Principal,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdatePrincipalInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Principal> {
    return this.runtime.patch<unknown>(`/principals/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Principal') as Principal);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/principals/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class TeamApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Team[]> {
    return this.runtime.get<unknown>("/teams", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Team') as Team[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Team> {
    return this.runtime.get<unknown>(`/teams/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Team') as Team);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Team>> {
    return this.runtime.getWithResponse<unknown>(`/teams/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Team') as Team,
      response: result.response,
    }));
  }

  create(input: CreateTeamInput, options: CratestackRequestConfig = {}): Promise<Team> {
    return this.runtime.post<unknown>("/teams", input, options)
      .then((value) => reviveWireFields(value, 'Team') as Team);
  }

  update(
    id: string,
    input: UpdateTeamInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Team> {
    return this.runtime.patch<unknown>(`/teams/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Team') as Team);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/teams/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class TeamMemberApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<TeamMember[]> {
    return this.runtime.get<unknown>("/team_members", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'TeamMember') as TeamMember[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<TeamMember> {
    return this.runtime.get<unknown>(`/team_members/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'TeamMember') as TeamMember);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<TeamMember>> {
    return this.runtime.getWithResponse<unknown>(`/team_members/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'TeamMember') as TeamMember,
      response: result.response,
    }));
  }

  create(input: CreateTeamMemberInput, options: CratestackRequestConfig = {}): Promise<TeamMember> {
    return this.runtime.post<unknown>("/team_members", input, options)
      .then((value) => reviveWireFields(value, 'TeamMember') as TeamMember);
  }

  update(
    id: string,
    input: UpdateTeamMemberInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<TeamMember> {
    return this.runtime.patch<unknown>(`/team_members/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'TeamMember') as TeamMember);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/team_members/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class ProjectApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Project[]> {
    return this.runtime.get<unknown>("/projects", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Project') as Project[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Project> {
    return this.runtime.get<unknown>(`/projects/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Project') as Project);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Project>> {
    return this.runtime.getWithResponse<unknown>(`/projects/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Project') as Project,
      response: result.response,
    }));
  }

  create(input: CreateProjectInput, options: CratestackRequestConfig = {}): Promise<Project> {
    return this.runtime.post<unknown>("/projects", input, options)
      .then((value) => reviveWireFields(value, 'Project') as Project);
  }

  update(
    id: string,
    input: UpdateProjectInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Project> {
    return this.runtime.patch<unknown>(`/projects/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Project') as Project);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/projects/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class TaskApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Task[]> {
    return this.runtime.get<unknown>("/tasks", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Task') as Task[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Task> {
    return this.runtime.get<unknown>(`/tasks/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Task') as Task);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Task>> {
    return this.runtime.getWithResponse<unknown>(`/tasks/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Task') as Task,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateTaskInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Task> {
    return this.runtime.patch<unknown>(`/tasks/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Task') as Task);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/tasks/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class ItemApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Item[]> {
    return this.runtime.get<unknown>("/items", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Item') as Item[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Item> {
    return this.runtime.get<unknown>(`/items/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Item') as Item);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Item>> {
    return this.runtime.getWithResponse<unknown>(`/items/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Item') as Item,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateItemInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Item> {
    return this.runtime.patch<unknown>(`/items/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Item') as Item);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/items/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class CommentApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Comment[]> {
    return this.runtime.get<unknown>("/comments", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Comment') as Comment[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Comment> {
    return this.runtime.get<unknown>(`/comments/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Comment') as Comment);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Comment>> {
    return this.runtime.getWithResponse<unknown>(`/comments/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Comment') as Comment,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateCommentInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Comment> {
    return this.runtime.patch<unknown>(`/comments/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Comment') as Comment);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/comments/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class TaskDepApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<TaskDep[]> {
    return this.runtime.get<unknown>("/task_deps", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'TaskDep') as TaskDep[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<TaskDep> {
    return this.runtime.get<unknown>(`/task_deps/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'TaskDep') as TaskDep);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<TaskDep>> {
    return this.runtime.getWithResponse<unknown>(`/task_deps/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'TaskDep') as TaskDep,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateTaskDepInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<TaskDep> {
    return this.runtime.patch<unknown>(`/task_deps/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'TaskDep') as TaskDep);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/task_deps/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class WakeTargetApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<WakeTarget[]> {
    return this.runtime.get<unknown>("/wake_targets", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'WakeTarget') as WakeTarget[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<WakeTarget> {
    return this.runtime.get<unknown>(`/wake_targets/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'WakeTarget') as WakeTarget);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<WakeTarget>> {
    return this.runtime.getWithResponse<unknown>(`/wake_targets/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'WakeTarget') as WakeTarget,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateWakeTargetInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<WakeTarget> {
    return this.runtime.patch<unknown>(`/wake_targets/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'WakeTarget') as WakeTarget);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/wake_targets/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class EventApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Event[]> {
    return this.runtime.get<unknown>("/events", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Event') as Event[]);
  }

  get(id: bigint, options: CratestackQueryRequestConfig = {}): Promise<Event> {
    return this.runtime.get<unknown>(`/events/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Event') as Event);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: bigint,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Event>> {
    return this.runtime.getWithResponse<unknown>(`/events/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Event') as Event,
      response: result.response,
    }));
  }

  update(
    id: bigint,
    input: UpdateEventInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Event> {
    return this.runtime.patch<unknown>(`/events/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Event') as Event);
  }

  delete(id: bigint, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/events/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class ProceduresApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  whoami(args: WhoamiArgs, options: CratestackRequestConfig = {}): Promise<WhoamiView> {
    return this.runtime.post<unknown>("/$procs/whoami", args, options)
      .then((value) => reviveWireFields(value, 'WhoamiView') as WhoamiView);
  }

  projectOverview(args: ProjectOverviewArgs, options: CratestackRequestConfig = {}): Promise<ProjectSummary[]> {
    return this.runtime.post<unknown>("/$procs/projectOverview", args, options)
      .then((value) => reviveWireFields(value, 'ProjectSummary') as ProjectSummary[]);
  }

  projectTasks(args: ProjectTasksArgs, options: CratestackRequestConfig = {}): Promise<TaskView[]> {
    return this.runtime.post<unknown>("/$procs/projectTasks", args, options)
      .then((value) => reviveWireFields(value, 'TaskView') as TaskView[]);
  }

  taskDetail(args: TaskDetailArgs, options: CratestackRequestConfig = {}): Promise<TaskDetail> {
    return this.runtime.post<unknown>("/$procs/taskDetail", args, options)
      .then((value) => reviveWireFields(value, 'TaskDetail') as TaskDetail);
  }

  getBoard(args: GetBoardArgs, options: CratestackRequestConfig = {}): Promise<MarkdownDoc> {
    return this.runtime.post<unknown>("/$procs/getBoard", args, options)
      .then((value) => reviveWireFields(value, 'MarkdownDoc') as MarkdownDoc);
  }

  getTaskMarkdown(args: GetTaskMarkdownArgs, options: CratestackRequestConfig = {}): Promise<MarkdownDoc> {
    return this.runtime.post<unknown>("/$procs/getTaskMarkdown", args, options)
      .then((value) => reviveWireFields(value, 'MarkdownDoc') as MarkdownDoc);
  }

  taskCreate(args: TaskCreateArgs, options: CratestackRequestConfig = {}): Promise<TaskDetail> {
    return this.runtime.post<unknown>("/$procs/taskCreate", args, options)
      .then((value) => reviveWireFields(value, 'TaskDetail') as TaskDetail);
  }

  taskEdit(args: TaskEditArgs, options: CratestackRequestConfig = {}): Promise<TaskDetail> {
    return this.runtime.post<unknown>("/$procs/taskEdit", args, options)
      .then((value) => reviveWireFields(value, 'TaskDetail') as TaskDetail);
  }

  claimNext(args: ClaimNextArgs, options: CratestackRequestConfig = {}): Promise<ClaimResult> {
    return this.runtime.post<unknown>("/$procs/claimNext", args, options)
      .then((value) => reviveWireFields(value, 'ClaimResult') as ClaimResult);
  }

  claimTask(args: ClaimTaskArgs, options: CratestackRequestConfig = {}): Promise<ClaimResult> {
    return this.runtime.post<unknown>("/$procs/claimTask", args, options)
      .then((value) => reviveWireFields(value, 'ClaimResult') as ClaimResult);
  }

  heartbeat(args: HeartbeatArgs, options: CratestackRequestConfig = {}): Promise<LeaseRenewal> {
    return this.runtime.post<unknown>("/$procs/heartbeat", args, options)
      .then((value) => reviveWireFields(value, 'LeaseRenewal') as LeaseRenewal);
  }

  checkItem(args: CheckItemArgs, options: CratestackRequestConfig = {}): Promise<TaskDetail> {
    return this.runtime.post<unknown>("/$procs/checkItem", args, options)
      .then((value) => reviveWireFields(value, 'TaskDetail') as TaskDetail);
  }

  comment(args: CommentArgs, options: CratestackRequestConfig = {}): Promise<OkResult> {
    return this.runtime.post<unknown>("/$procs/comment", args, options)
      .then((value) => reviveWireFields(value, 'OkResult') as OkResult);
  }

  complete(args: CompleteArgs, options: CratestackRequestConfig = {}): Promise<TaskView> {
    return this.runtime.post<unknown>("/$procs/complete", args, options)
      .then((value) => reviveWireFields(value, 'TaskView') as TaskView);
  }

  release(args: ReleaseArgs, options: CratestackRequestConfig = {}): Promise<TaskView> {
    return this.runtime.post<unknown>("/$procs/release", args, options)
      .then((value) => reviveWireFields(value, 'TaskView') as TaskView);
  }

  block(args: BlockArgs, options: CratestackRequestConfig = {}): Promise<TaskView> {
    return this.runtime.post<unknown>("/$procs/block", args, options)
      .then((value) => reviveWireFields(value, 'TaskView') as TaskView);
  }

  changesSince(args: ChangesSinceArgs, options: CratestackRequestConfig = {}): Promise<ChangeFeed> {
    return this.runtime.post<unknown>("/$procs/changesSince", args, options)
      .then((value) => reviveWireFields(value, 'ChangeFeed') as ChangeFeed);
  }

  waitForWork(args: WaitForWorkArgs, options: CratestackRequestConfig = {}): Promise<WaitResult> {
    return this.runtime.post<unknown>("/$procs/waitForWork", args, options)
      .then((value) => reviveWireFields(value, 'WaitResult') as WaitResult);
  }

  unblockTask(args: UnblockTaskArgs, options: CratestackRequestConfig = {}): Promise<TaskView> {
    return this.runtime.post<unknown>("/$procs/unblockTask", args, options)
      .then((value) => reviveWireFields(value, 'TaskView') as TaskView);
  }

  cancelTask(args: CancelTaskArgs, options: CratestackRequestConfig = {}): Promise<TaskView> {
    return this.runtime.post<unknown>("/$procs/cancelTask", args, options)
      .then((value) => reviveWireFields(value, 'TaskView') as TaskView);
  }

}