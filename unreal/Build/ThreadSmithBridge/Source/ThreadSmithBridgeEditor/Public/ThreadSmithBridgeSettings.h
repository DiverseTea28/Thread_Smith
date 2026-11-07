// Editor Preferences > Plugins > ThreadSmith Bridge.

#pragma once

#include "CoreMinimal.h"
#include "Engine/DeveloperSettings.h"
#include "ThreadSmithBridgeSettings.generated.h"

UCLASS(config = EditorPerProjectUserSettings, defaultconfig, meta = (DisplayName = "ThreadSmith Bridge"))
class UThreadSmithBridgeSettings : public UDeveloperSettings
{
	GENERATED_BODY()

public:
	UThreadSmithBridgeSettings();

	virtual FName GetContainerName() const override { return TEXT("Editor"); }
	virtual FName GetCategoryName() const override { return TEXT("Plugins"); }

	/** ThreadSmith's outfit library. Empty = Documents/ThreadSmith/Library. */
	UPROPERTY(config, EditAnywhere, Category = "Library")
	FDirectoryPath LibraryFolder;

	/** threadsmith.exe, so the plugin can start ThreadSmith when it isn't running. */
	UPROPERTY(config, EditAnywhere, Category = "ThreadSmith App", meta = (FilePathFilter = "exe"))
	FFilePath ThreadSmithExe;

	/** Where imported outfits go in the project. */
	UPROPERTY(config, EditAnywhere, Category = "Import", meta = (ContentDir))
	FString OutfitContentFolder = TEXT("/Game/ThreadSmith/Outfits");

	/** Port ThreadSmith listens on (must match the app). */
	UPROPERTY(config, EditAnywhere, Category = "Connection", AdvancedDisplay)
	int32 AppPort = 47830;

	/** Port this plugin listens on (must match the app). */
	UPROPERTY(config, EditAnywhere, Category = "Connection", AdvancedDisplay)
	int32 PluginPort = 47831;

	/** Full path of the library folder (applies the default). */
	FString GetLibraryFolder() const;
};
