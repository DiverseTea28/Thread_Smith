// Copyright Epic Games, Inc. All Rights Reserved.
/*===========================================================================
	Generated code exported from UnrealHeaderTool.
	DO NOT modify this manually! Edit the corresponding .h files instead!
===========================================================================*/

#include "UObject/GeneratedCppIncludes.h"
#include "ThreadSmithOutfitComponent.h"

PRAGMA_DISABLE_DEPRECATION_WARNINGS
static_assert(!UE_WITH_CONSTINIT_UOBJECT, "This generated code can only be compiled with !UE_WITH_CONSTINIT_UOBJECT");
void EmptyLinkFunctionForGeneratedCodeThreadSmithOutfitComponent() {}

// ********** Begin Cross Module References ********************************************************
ENGINE_API UClass* Z_Construct_UClass_USkeletalMeshComponent(ETypeConstructPhase);
// ********** End Cross Module References **********************************************************

// ********** Begin Same Module References *********************************************************
UPackage* Z_Construct_UPackage__Script_ThreadSmithRuntime(ETypeConstructPhase);
THREADSMITHRUNTIME_API UClass* Z_Construct_UClass_UThreadSmithOutfitComponent(ETypeConstructPhase);
THREADSMITHRUNTIME_API UClass* Z_Construct_UClass_UThreadSmithOutfitComponent(ETypeConstructPhase);
// ********** End Same Module References ***********************************************************
#define UHT_STRUCT_BASE(INIT) UE::CodeGen::ConstInit::TCompiledInObjectPtr<const FStructBaseChain>(UE::Private::AsStructBaseChain(INIT))

// ********** Begin Class UThreadSmithOutfitComponent Function FollowBody **************************
#ifdef UHT_STATICS
#error UHT_STATICS already defined
#endif
#define UHT_STATICS Z_Construct_UFunction_UThreadSmithOutfitComponent_FollowBody_Statics
struct UHT_STATICS
{
#if WITH_METADATA
	static constexpr UECodeGen_Private::FMetaDataPairParam Type_MetaData[] = {
		{ "Category", "ThreadSmith" },
		{ "Comment", "/** Find the body and follow its pose. Called automatically when the component registers. */" },
		{ "ModuleRelativePath", "Public/ThreadSmithOutfitComponent.h" },
		{ "ToolTip", "Find the body and follow its pose. Called automatically when the component registers." },
	};
#endif // WITH_METADATA

// ********** Begin Function FollowBody constinit property declarations ****************************
// ********** End Function FollowBody constinit property declarations ******************************
	static const UECodeGen_Private::FFunctionParams FuncParams;
};
const UECodeGen_Private::FFunctionParams UHT_STATICS::FuncParams = { { (FTypeConstructFunc*)Z_Construct_UClass_UThreadSmithOutfitComponent, nullptr, "FollowBody", nullptr, 0, 0, RF_Public|RF_Transient|RF_MarkAsNative, (EFunctionFlags)0x04020401, 0, 0, METADATA_PARAMS(UE_ARRAY_COUNT(UHT_STATICS::Type_MetaData), UHT_STATICS::Type_MetaData)},  };
UFunction* Z_Construct_UFunction_UThreadSmithOutfitComponent_FollowBody(ETypeConstructPhase Phase)
{
	static UFunction* ReturnFunction = nullptr;
	if (!ReturnFunction)
	{
		UECodeGen_Private::ConstructUFunction(&ReturnFunction, UHT_STATICS::FuncParams);
	}
	return ReturnFunction;
}
#undef UHT_STATICS
DEFINE_FUNCTION(UThreadSmithOutfitComponent::execFollowBody)
{
	P_FINISH;
	P_NATIVE_BEGIN;
	P_THIS->FollowBody();
	P_NATIVE_END;
}
// ********** End Class UThreadSmithOutfitComponent Function FollowBody ****************************

// ********** Begin Class UThreadSmithOutfitComponent **********************************************
#ifdef UHT_STATICS
#error UHT_STATICS already defined
#endif
#define UHT_STATICS Z_Construct_UClass_UThreadSmithOutfitComponent_Statics
struct UHT_STATICS
{
#if WITH_METADATA
	static constexpr UECodeGen_Private::FMetaDataPairParam Type_MetaData[] = {
		{ "BlueprintSpawnableComponent", "" },
		{ "ClassGroupNames", "ThreadSmith" },
		{ "HideCategories", "Physics Collision Object Mesh|SkeletalAsset Object Mobility Trigger" },
		{ "IncludePath", "ThreadSmithOutfitComponent.h" },
		{ "ModuleRelativePath", "Public/ThreadSmithOutfitComponent.h" },
	};
	static constexpr UECodeGen_Private::FMetaDataPairParam NewProp_BodyComponentName_MetaData[] = {
		{ "Category", "ThreadSmith" },
		{ "Comment", "/** Name of the body component to follow. Empty = follow the component this is attached to. */" },
		{ "ModuleRelativePath", "Public/ThreadSmithOutfitComponent.h" },
		{ "ToolTip", "Name of the body component to follow. Empty = follow the component this is attached to." },
	};
	static constexpr UECodeGen_Private::FMetaDataPairParam NewProp_OutfitId_MetaData[] = {
		{ "Category", "ThreadSmith" },
		{ "Comment", "/** Library id of the outfit (from outfit.json), so the Wardrobe can find it again. */" },
		{ "ModuleRelativePath", "Public/ThreadSmithOutfitComponent.h" },
		{ "ToolTip", "Library id of the outfit (from outfit.json), so the Wardrobe can find it again." },
	};
#endif // WITH_METADATA

// ********** Begin Class UThreadSmithOutfitComponent constinit property declarations **************
	static const UECodeGen_Private::FNamePropertyParams NewProp_BodyComponentName;
	static const UECodeGen_Private::FStrPropertyParams NewProp_OutfitId;
	static const UECodeGen_Private::FPropertyParamsBase* const PropPointers[];
// ********** End Class UThreadSmithOutfitComponent constinit property declarations ****************
	static constexpr UE::CodeGen::FClassNativeFunction Funcs[] = {
		{ .NameUTF8 = UTF8TEXT("FollowBody"), .Pointer = &UThreadSmithOutfitComponent::execFollowBody },
	};
	static FTypeConstructFunc* DependentSingletons[];
	static constexpr FClassFunctionLinkInfo FuncInfo[] = {
		{ &Z_Construct_UFunction_UThreadSmithOutfitComponent_FollowBody, "FollowBody" }, // 60baf99a5abec05980726837fb456f778ea67fb2
	};
	static_assert(UE_ARRAY_COUNT(FuncInfo) < 2048);
	static constexpr FCppClassTypeInfoStatic StaticCppClassTypeInfo = {
		TCppClassTypeTraits<UThreadSmithOutfitComponent>::IsAbstract,
	};
	static const UECodeGen_Private::FClassParams ClassParams;
}; // struct UHT_STATICS

// ********** Begin Class UThreadSmithOutfitComponent Property Definitions *************************
const UECodeGen_Private::FNamePropertyParams UHT_STATICS::NewProp_BodyComponentName = { "BodyComponentName", nullptr, (EPropertyFlags)0x0010000000000005, UECodeGen_Private::EPropertyGenFlags::Name, nullptr, nullptr, 1, STRUCT_OFFSET(UThreadSmithOutfitComponent, BodyComponentName), METADATA_PARAMS(UE_ARRAY_COUNT(NewProp_BodyComponentName_MetaData), NewProp_BodyComponentName_MetaData) };
const UECodeGen_Private::FStrPropertyParams UHT_STATICS::NewProp_OutfitId = { "OutfitId", nullptr, (EPropertyFlags)0x0010000000020015, UECodeGen_Private::EPropertyGenFlags::Str, nullptr, nullptr, 1, STRUCT_OFFSET(UThreadSmithOutfitComponent, OutfitId), METADATA_PARAMS(UE_ARRAY_COUNT(NewProp_OutfitId_MetaData), NewProp_OutfitId_MetaData) };
const UECodeGen_Private::FPropertyParamsBase* const UHT_STATICS::PropPointers[] = {
	(const UECodeGen_Private::FPropertyParamsBase*)&UHT_STATICS::NewProp_BodyComponentName,
	(const UECodeGen_Private::FPropertyParamsBase*)&UHT_STATICS::NewProp_OutfitId,
};
static_assert(UE_ARRAY_COUNT(UHT_STATICS::PropPointers) < 2048);
// ********** End Class UThreadSmithOutfitComponent Property Definitions ***************************
FTypeConstructFunc* UHT_STATICS::DependentSingletons[] = {
	(FTypeConstructFunc*)Z_Construct_UClass_USkeletalMeshComponent,
	(FTypeConstructFunc*)Z_Construct_UPackage__Script_ThreadSmithRuntime,
};
static_assert(UE_ARRAY_COUNT(UHT_STATICS::DependentSingletons) < 16);
const UECodeGen_Private::FClassParams UHT_STATICS::ClassParams = {
	&Z_Construct_UClass_UThreadSmithOutfitComponent,
	"Engine",
	&StaticCppClassTypeInfo,
	DependentSingletons,
	FuncInfo,
	UHT_STATICS::PropPointers,
	nullptr,
	UE_ARRAY_COUNT(DependentSingletons),
	UE_ARRAY_COUNT(FuncInfo),
	UE_ARRAY_COUNT(UHT_STATICS::PropPointers),
	0,
	0x00B010A4u,
	METADATA_PARAMS(UE_ARRAY_COUNT(UHT_STATICS::Type_MetaData), UHT_STATICS::Type_MetaData)
};
static void UThreadSmithOutfitComponent_StaticRegisterNativesUThreadSmithOutfitComponent()
{
	UClass* Class = UThreadSmithOutfitComponent::StaticClass();
	FNativeFunctionRegistrar::RegisterFunctions(Class, 		MakeConstArrayView(UHT_STATICS::Funcs));
}
FClassRegistrationInfo Z_Registration_Info_UClass_UThreadSmithOutfitComponent;
UClass* Z_Construct_UClass_UThreadSmithOutfitComponent(ETypeConstructPhase Phase)
{
	if (Phase == ETypeConstructPhase::Inner)
	{
		using TClass = UThreadSmithOutfitComponent;
		if (!Z_Registration_Info_UClass_UThreadSmithOutfitComponent.InnerSingleton)
		{
			GetPrivateStaticClassBody(
				TClass::StaticPackage(),
				TEXT("ThreadSmithOutfitComponent"),
				Z_Registration_Info_UClass_UThreadSmithOutfitComponent.InnerSingleton,
				UThreadSmithOutfitComponent_StaticRegisterNativesUThreadSmithOutfitComponent,
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
		return Z_Registration_Info_UClass_UThreadSmithOutfitComponent.InnerSingleton;
	}
	if (!Z_Registration_Info_UClass_UThreadSmithOutfitComponent.OuterSingleton)
	{
		UECodeGen_Private::ConstructUClass(Z_Registration_Info_UClass_UThreadSmithOutfitComponent.OuterSingleton, UHT_STATICS::ClassParams);
	}
	return Z_Registration_Info_UClass_UThreadSmithOutfitComponent.OuterSingleton;
}
#undef UHT_STATICS
DEFINE_VTABLE_PTR_HELPER_CTOR_NS(, UThreadSmithOutfitComponent);
UThreadSmithOutfitComponent::~UThreadSmithOutfitComponent() {}
// ********** End Class UThreadSmithOutfitComponent ************************************************

// ********** Begin Registration *******************************************************************
#ifdef UHT_STATICS
#error UHT_STATICS already defined
#endif
#define UHT_STATICS Z_CompiledInDeferFile_FID_Users_tyco0_Documents_ThreadSmith_unreal_Build_ThreadSmithBridge_HostProject_Plugins_ThreadSmithBridge_Source_ThreadSmithRuntime_Public_ThreadSmithOutfitComponent_h__Script_ThreadSmithRuntime_Statics
struct UHT_STATICS
{
	static constexpr FClassRegisterCompiledInInfo ClassInfo[] = {
		{ Z_Construct_UClass_UThreadSmithOutfitComponent, TEXT("UThreadSmithOutfitComponent"), &Z_Registration_Info_UClass_UThreadSmithOutfitComponent, CONSTRUCT_RELOAD_VERSION_INFO(FClassReloadVersionInfo, sizeof(UThreadSmithOutfitComponent), 1175346878U) },
	};
}; // UHT_STATICS 
static FRegisterCompiledInInfo Z_CompiledInDeferFile_FID_Users_tyco0_Documents_ThreadSmith_unreal_Build_ThreadSmithBridge_HostProject_Plugins_ThreadSmithBridge_Source_ThreadSmithRuntime_Public_ThreadSmithOutfitComponent_h__Script_ThreadSmithRuntime_4cf7c8d92c1ecfefda5b4e7aed82b18fc5a13d91{
	TEXT("/Script/ThreadSmithRuntime"),
	UHT_STATICS::ClassInfo, UE_ARRAY_COUNT(UHT_STATICS::ClassInfo),
	nullptr, 0,
	nullptr, 0,
	nullptr, 0,
};
#undef UHT_STATICS
// ********** End Registration *********************************************************************
#undef UHT_STRUCT_BASE

PRAGMA_ENABLE_DEPRECATION_WARNINGS
