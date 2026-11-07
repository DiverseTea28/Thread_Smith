#include "ThreadSmithBridgeSettings.h"

#include "HAL/PlatformProcess.h"
#include "Misc/Paths.h"

UThreadSmithBridgeSettings::UThreadSmithBridgeSettings()
{
	// Default location of the ThreadSmith build in Documents.
	ThreadSmithExe.FilePath = FPaths::Combine(FPlatformProcess::UserDir(), TEXT("ThreadSmith/target/release/threadsmith.exe"));
}

FString UThreadSmithBridgeSettings::GetLibraryFolder() const
{
	if (!LibraryFolder.Path.IsEmpty())
	{
		return LibraryFolder.Path;
	}
	// FPlatformProcess::UserDir() is the user's Documents folder on Windows.
	return FPaths::Combine(FPlatformProcess::UserDir(), TEXT("ThreadSmith/Library"));
}
