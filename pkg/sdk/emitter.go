package sdk

// Emitter compiles a target-agnostic resource tree into one target's native
// syntax.
//
// It is deliberately separate from a runner: Emit has no side effects, which is
// what lets `meridian plan` be a pure function. A plugin that needs to shell out
// to a real CLI does so behind a different interface, outside the compile chain.
type Emitter interface {
	// Name is the target name documents select with spec.target.
	Name() string
	// SupportedResourceTypes lists every IR resource type this target maps,
	// sorted. Anything outside it is a compile error, never a best-effort guess.
	SupportedResourceTypes() []string
	// Capabilities declares what this target can express natively.
	Capabilities() Capabilities
	// Emit compiles the tree. Warnings describe what the target could not
	// preserve; an error means it could not compile at all, and should be a
	// *CompileError so the host can report it against the author's own source.
	Emit(g *ResourceGraph, data Data) (Artifact, []Warning, error)
}

// Category is the kind of tool a plugin drives. The core routes on it; it is a
// declared fact, never a reason for the core to special-case a product.
type Category int

const (
	CategoryUnspecified Category = iota
	// CategoryInfrastructureAsCode provisions infrastructure.
	CategoryInfrastructureAsCode
	// CategoryConfigurationManagement configures a node that already exists.
	CategoryConfigurationManagement
	// CategoryContainerization builds and runs containers on one host.
	CategoryContainerization
	// CategoryContainerOrchestration schedules containers across a cluster.
	CategoryContainerOrchestration
)

// Categorized is implemented by an Emitter that declares its category. It is a
// separate interface so a plugin written before categories existed still
// compiles; such a plugin reports CategoryUnspecified.
//
// A Go plugin compiles only: this SDK does not serve the Runner half of the
// protocol, so it never declares an operation beyond compiling.
type Categorized interface {
	Category() Category
}
