// Copyright Epic Games, Inc. All Rights Reserved.
/*===========================================================================
	Generated code exported from UnrealHeaderTool.
	DO NOT modify this manually! Edit the corresponding .h files instead!
===========================================================================*/

#include "UObject/GeneratedCppIncludes.h"
#include "ThreadSmithBridgeSettings.h"
#include "UObject/SoftObjectPath.h"

PRAGMA_DISABLE_DEPRECATION_WARNINGS
static_assert(!UE_WITH_CONSTINIT_UOBJECT, "This generated code can only be compiled with !UE_WITH_CONSTINIT_UOBJECT");
void EmptyLinkFunctionForGeneratedCodeThreadSmithBridgeSettings() {}

// ********** Begin Cross Module References ********************************************************
COREUOBJECT_API UScriptStruct* Z_Construct_UScriptStruct_FDirectoryPath(ETypeConstructPhase);
COREUOBJECT_API UScriptStruct* Z_Construct_UScriptStruct_FFilePath(ETypeConstructPhase);
DEVELOPERSETTINGS_API UClass* Z_Construct_UClass_UDeveloperSettings(ETypeConstructPhase);
// ********** End Cross Module References **********************************************************

// ********** Begin Same Module References *********************************************************
UPackage* Z_Construct_UPackage__Script_ThreadSmithBridgeEditor(ETypeConstructPhase);
THREADSMITHBRIDGEEDITOR_API UClass* Z_Construct_UClass_UThreadSmithBridgeSettings(ETypeConstructPhase);
THREADSMITHBRIDGEEDITOR_API UClass* Z_Construct_UClass_UThreadSmithBridgeSettings(ETypeConstructPhase);
// ********** End Same Module References ***********************************************************
#define UHT_STRUCT_BASE(INIT) UE::CodeGen::ConstInit::TCompiledInObjectPtr<const FStructBaseChain>(UE::Private::AsStructBaseChain(INIT))

// ********** Begin Class UThreadSmithBridgeSettings ***********************************************
#ifdef UHT_STATICS
#error UHT_STATICS already defined
#endif
#define UHT_STATICS Z_Construct_UClass_UThreadSmithBridgeSettings_Statics
struct UHT_STATICS
{
#if WITH_METADATA
	static constexpr UECodeGen_Private::FMetaDataPairParam Type_MetaData[] = {
		{ "DisplayName", "ThreadSmith Bridge" },
		{ "IncludePath", "ThreadSmithBridgeSettings.h" },
		{ "ModuleRelativePath", "Public/ThreadSmithBridgeSettings.h" },
	};
	static constexpr UECodeGen_Private::FMetaDataPairParam NewProp_LibraryFolder_MetaData[] = {
		{ "Category", "Library" },
		{ "Comment", "/** ThreadSmith's outfit library. Empty = Documents/ThreadSmith/Library. */" },
		{ "ModuleRelativePath", "Public/ThreadSmithBridgeSettings.h" },
		{ "ToolTip", "ThreadSmith's outfit library. Empty = Documents/ThreadSmith/Library." },
	};
	static constexpr UECodeGen_Private::FMetaDataPairParam NewProp_ThreadSmithExe_MetaData[] = {
		{ "Category", "ThreadSmith App" },
		{ "Comment", "/** threadsmith.exe, so the plugin can start ThreadSmith when it isn't running. */" },
		{ "FilePathFilter", "exe" },
		{ "ModuleRelativePath", "Public/ThreadSmithBridgeSettings.h" },
		{ "ToolTip", "threadsmith.exe, so the plugin can start ThreadSmith when it isn't running." },
	};
	static constexpr UECodeGen_Private::FMetaDataPairParam NewProp_OutfitContentFolder_MetaData[] = {
		{ "Category", "Import" },
		{ "Comment", "/** Where imported outfits go in the project. */" },
		{ "ContentDir", "" },
		{ "ModuleRelativePath", "Public/ThreadSmithBridgeSettings.h" },
		{ "ToolTip", "Where imported outfits go in the project." },
	};
	static constexpr UECodeGen_Private::FMetaDataPairParam NewProp_AppPort_MetaData[] = {
		{ "Category", "Connection" },
		{ "Comment", "/** Port ThreadSmith listens on (must match the app). */" },
		{ "ModuleRelativePath", "Public/ThreadSmithBridgeSettings.h" },
		{ "ToolTip", "Port ThreadSmith listens on (must match the app)." },
	};
	static constexpr UECodeGen_Private::FMetaDataPairParam NewProp_PluginPort_MetaData[] = {
		{ "Category", "Connection" },
		{ "Comment", "/** Port this plugin listens on (must match the app). */" },
		{ "ModuleRelativePath", "Public/ThreadSmithBridgeSettings.h" },
		{ "ToolTip", "Port this plugin listens on (must match the app)." },
	};
#endif // WITH_METADATA

// ********** Begin Class UThreadSmithBridgeSettings constinit property declarations ***************
	static const UECodeGen_Private::FStructPropertyParams NewProp_LibraryFolder;
	static const UECodeGen_Private::FStructPropertyParams NewProp_ThreadSmithExe;
	static const UECodeGen_Private::FStrPropertyParams NewProp_OutfitContentFolder;
	static const UECodeGen_Private::FIntPropertyParams NewProp_AppPort;
	static const UECodeGen_Private::FIntPropertyParams NewProp_PluginPort;
	static const UECodeGen_Private::FPropertyParamsBase* const PropPointers[];
// ********** End Class UThreadSmithBridgeSettings constinit property declarations *****************
	static FTypeConstructFunc* DependentSingletons[];
	static constexpr FCppClassTypeInfoStatic StaticCppClassTypeInfo = {
		TCppClassTypeTraits<UThreadSmithBridgeSettings>::IsAbstract,
	};
	static const UECodeGen_Private::FClassParams ClassParams;
}; // struct UHT_STATICS

// ********** Begin Class UThreadSmithBridgeSettings Property Definitions **************************
const UECodeGen_Private::FStructPropertyParams UHT_STATICS::NewProp_LibraryFolder = { "LibraryFolder", nullptr, (EPropertyFlags)0x0010000000004001, UECodeGen_Private::EPropertyGenFlags::Struct, nullptr, nullptr, 1, STRUCT_OFFSET(UThreadSmithBridgeSettings, LibraryFolder), Z_Construct_UScriptStruct_FDirectoryPath, METADATA_PARAMS(UE_ARRAY_COUNT(NewProp_LibraryFolder_MetaData), NewProp_LibraryFolder_MetaData) }; // a9ea974572ee54d1b93cb49a6434fdbad40b6bf5
const UECodeGen_Private::FStructPropertyParams UHT_STATICS::NewProp_ThreadSmithExe = { "ThreadSmithExe", nullptr, (EPropertyFlags)0x0010000000004001, UECodeGen_Private::EPropertyGenFlags::Struct, nullptr, nullptr, 1, STRUCT_OFFSET(UThreadSmithBridgeSettings, ThreadSmithExe), Z_Construct_UScriptStruct_FFilePath, METADATA_PARAMS(UE_ARRAY_COUNT(NewProp_ThreadSmithExe_MetaData), NewProp_ThreadSmithExe_MetaData) }; // 0b92f70c4aaca653170512ccb353f3aa009da14a
const UECodeGen_Private::FStrPropertyParams UHT_STATICS::NewProp_OutfitContentFolder = { "OutfitContentFolder", nullptr, (EPropertyFlags)0x0010000000004001, UECodeGen_Private::EPropertyGenFlags::Str, nullptr, nullptr, 1, STRUCT_OFFSET(UThreadSmithBridgeSettings, OutfitContentFolder), METADATA_PARAMS(UE_ARRAY_COUNT(NewProp_OutfitContentFolder_MetaData), NewProp_OutfitContentFolder_MetaData) };
const UECodeGen_Private::FIntPropertyParams UHT_STATICS::NewProp_AppPort = { "AppPort", nullptr, (EPropertyFlags)0x0010040000004001, UECodeGen_Private::EPropertyGenFlags::Int, nullptr, nullptr, 1, STRUCT_OFFSET(UThreadSmithBridgeSettings, AppPort), METADATA_PARAMS(UE_ARRAY_COUNT(NewProp_AppPort_MetaData), NewProp_AppPort_MetaData) };
const UECodeGen_Private::FIntPropertyParams UHT_STATICS::NewProp_PluginPort = { "PluginPort", nullptr, (EPropertyFlags)0x0010040000004001, UECodeGen_Private::EPropertyGenFlags::Int, nullptr, nullptr, 1, STRUCT_OFFSET(UThreadSmithBridgeSettings, PluginPort), METADATA_PARAMS(UE_ARRAY_COUNT(NewProp_PluginPort_MetaData), NewProp_PluginPort_MetaData) };
const UECodeGen_Private::FPropertyParamsBase* const UHT_STATICS::PropPointers[] = {
	(const UECodeGen_Private::FPropertyParamsBase*)&UHT_STATICS::NewProp_LibraryFolder,
	(const UECodeGen_Private::FPropertyParamsBase*)&UHT_STATICS::NewProp_ThreadSmithExe,
	(const UECodeGen_Private::FPropertyParamsBase*)&UHT_STATICS::NewProp_OutfitContentFolder,
	(const UECodeGen_Private::FPropertyParamsBase*)&UHT_STATICS::NewProp_AppPort,
	(const UECodeGen_Private::FPropertyParamsBase*)&UHT_STATICS::NewProp_PluginPort,
};
static_assert(UE_ARRAY_COUNT(UHT_STATICS::PropPointers) < 2048);
// ********** End Class UThreadSmithBridgeSettings Property Definitions ****************************
FTypeConstructFunc* UHT_STATICS::DependentSingletons[] = {
	(FTypeConstructFunc*)Z_Construct_UClass_UDeveloperSettings,
	(FTypeConstructFunc*)Z_Construct_UPackage__Script_ThreadSmithBridgeEditor,
};
static_assert(UE_ARRAY_COUNT(UHT_STATICS::DependentSingletons) < 16);
const UECodeGen_Private::FClassParams UHT_STATICS::ClassParams = {
	&Z_Construct_UClass_UThreadSmithBridgeSettings,
	"EditorPerProjectUserSettings",
	&StaticCppClassTypeInfo,
	DependentSingletons,
	nullptr,
	UHT_STATICS::PropPointers,
	nullptr,
	UE_ARRAY_COUNT(DependentSingletons),
	0,
	UE_ARRAY_COUNT(UHT_STATICS::PropPointers),
	0,
	0x000000A6u,
	METADATA_PARAMS(UE_ARRAY_COUNT(UHT_STATICS::Type_MetaData), UHT_STATICS::Type_MetaData)
};
FClassRegistrationInfo Z_Registration_Info_UClass_UThreadSmithBridgeSettings;
UClass* Z_Construct_UClass_UThreadSmithBridgeSettings(ETypeConstructPhase Phase)
{
	if (Phase == ETypeConstructPhase::Inner)
	{
		using TClass = UThreadSmithBridgeSettings;
		if (!Z_Registration_Info_UClass_UThreadSmithBridgeSettings.InnerSingleton)
		{
			GetPrivateStaticClassBody(
				TClass::StaticPackage(),
				TEXT("ThreadSmithBridgeSettings"),
				Z_Registration_Info_UClass_UThreadSmithBridgeSettings.InnerSingleton,
				nullptr,
				DataSizeOf<TClass>(),
				alignof(TClass),
				TClass::StaticClassFlags,
				TClass::StaticClassCastFlags(),
				TClass::StaticConfigName(),
				(UClass::ClassConstructorType)InternalConstructor<TClass>,
				(UClass::ClassVTableHelperCtorCallerType)InternalVTableHelperCtorCaller<TClass>,
				UOBJECT_CPPCLASS_STATICFUNCTIONS_FORCLASS(TClass),
				&TClass::Super::StaticClass,
				&TClass::WithinClass::StaticClass
			);
		}
		return Z_Registration_Info_UClass_UThreadSmithBridgeSettings.InnerSingleton;
	}
	if (!Z_Registration_Info_UClass_UThreadSmithBridgeSettings.OuterSingleton)
	{
		UECodeGen_Private::ConstructUClass(Z_Registration_Info_UClass_UThreadSmithBridgeSettings.OuterSingleton, UHT_STATICS::ClassParams);
	}
	return Z_Registration_Info_UClass_UThreadSmithBridgeSettings.OuterSingleton;
}
#undef UHT_STATICS
DEFINE_VTABLE_PTR_HELPER_CTOR_NS(, UThreadSmithBridgeSettings);
UThreadSmithBridgeSettings::~UThreadSmithBridgeSettings() {}
// ********** End Class UThreadSmithBridgeSettings *************************************************

// ********** Begin Registration *******************************************************************
#ifdef UHT_STATICS
#error UHT_STATICS already defined
#endif
#define UHT_STATICS Z_CompiledInDeferFile_FID_Users_tyco0_Documents_ThreadSmith_unreal_Build_ThreadSmithBridge_HostProject_Plugins_ThreadSmithBridge_Source_ThreadSmithBridgeEditor_Public_ThreadSmithBridgeSettings_h__Script_ThreadSmithBridgeEditor_Statics
struct UHT_STATICS
{
	static constexpr FClassRegisterCompiledInInfo ClassInfo[] = {
		{ Z_Construct_UClass_UThreadSmithBridgeSettings, TEXT("UThreadSmithBridgeSettings"), &Z_Registration_Info_UClass_UThreadSmithBridgeSettings, CONSTRUCT_RELOAD_VERSION_INFO(FClassReloadVersionInfo, sizeof(UThreadSmithBridgeSettings), 3440628027U) },
	};
}; // UHT_STATICS 
static FRegisterCompiledInInfo Z_CompiledInDeferFile_FID_Users_tyco0_Documents_ThreadSmith_unreal_Build_ThreadSmithBridge_HostProject_Plugins_ThreadSmithBridge_Source_ThreadSmithBridgeEditor_Public_ThreadSmithBridgeSettings_h__Script_ThreadSmithBridgeEditor_d2c3d333198f21b0283d6baa206c502cb32a93a3{
	TEXT("/Script/ThreadSmithBridgeEditor"),
	UHT_STATICS::ClassInfo, UE_ARRAY_COUNT(UHT_STATICS::ClassInfo),
	nullptr, 0,
	nullptr, 0,
	nullptr, 0,
};
#undef UHT_STATICS
// ********** End Registration *********************************************************************
#undef UHT_STRUCT_BASE

PRAGMA_ENABLE_DEPRECATION_WARNINGS
