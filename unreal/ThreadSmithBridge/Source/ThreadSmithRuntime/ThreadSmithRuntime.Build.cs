// ThreadSmith runtime module: the outfit component that follows the body.
// Kept tiny so it can ship with games.

using UnrealBuildTool;

public class ThreadSmithRuntime : ModuleRules
{
	public ThreadSmithRuntime(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;

		PublicDependencyModuleNames.AddRange(new string[]
		{
			"Core",
			"CoreUObject",
			"Engine",
		});
	}
}
